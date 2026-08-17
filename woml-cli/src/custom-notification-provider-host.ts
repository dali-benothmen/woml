#!/usr/bin/env bun

import { randomUUID } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { Worker } from 'node:worker_threads';

import {
  assertCustomProviderInbound,
  CUSTOM_NOTIFICATION_PROVIDER_MAX_FRAME_BYTES,
  CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL,
  CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
  type CustomProviderCompletedMessage,
  type CustomProviderExecuteMessage,
  type CustomProviderFailureKind,
  type CustomProviderReceipt,
} from './custom-notification-provider';
import { FrameDecoder, SerializedFrameWriter } from './script-host/framing';

interface ArtifactManifest {
  readonly artifacts: readonly {
    readonly scriptArtifactId: string;
    readonly source: string;
  }[];
}

interface WorkerResponse {
  readonly ok: boolean;
  readonly receipt?: CustomProviderReceipt;
  readonly kind?: CustomProviderFailureKind;
  readonly message?: string;
  readonly retryable?: boolean;
}

interface ActiveInvocation {
  readonly worker: Worker;
  readonly startedAt: number;
  readonly timer: ReturnType<typeof setTimeout>;
}

function completed(
  invocationId: string,
  startedAt: number,
  outcome: CustomProviderCompletedMessage['outcome']
): CustomProviderCompletedMessage {
  return {
    protocol: CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL,
    protocolVersion: CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
    messageType: 'completed',
    invocationId,
    durationMs: Math.max(0, performance.now() - startedAt),
    outcome,
  };
}

function failure(
  invocationId: string,
  startedAt: number,
  kind: CustomProviderFailureKind,
  code: string,
  message: string,
  retryable: boolean
): CustomProviderCompletedMessage {
  return completed(invocationId, startedAt, {
    kind: 'failed',
    error: { kind, code, message: message.slice(0, 1024), retryable },
  });
}

function workerPath(): URL {
  return new URL(
    import.meta.url.endsWith('.ts')
      ? './custom-notification-provider/worker.ts'
      : './custom-notification-provider-worker.js',
    import.meta.url
  );
}

async function writeStdout(frame: Uint8Array): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    process.stdout.write(frame, error =>
      error === null || error === undefined ? resolve() : reject(error)
    );
  });
}

export async function runCustomNotificationProviderHost(
  manifestPath = process.argv[2]
): Promise<void> {
  if (manifestPath === undefined) {
    throw new Error('Custom provider host requires an immutable artifact manifest.');
  }
  const manifest = JSON.parse(await readFile(manifestPath, 'utf8')) as ArtifactManifest;
  const artifacts = new Map(
    manifest.artifacts.map(artifact => [artifact.scriptArtifactId, artifact.source])
  );
  const writer = new SerializedFrameWriter(writeStdout);
  const decoder = new FrameDecoder({
    maxFrameBytes: CUSTOM_NOTIFICATION_PROVIDER_MAX_FRAME_BYTES,
  });
  const active = new Map<string, ActiveInvocation>();

  const execute = async (message: CustomProviderExecuteMessage) => {
    const source = artifacts.get(message.scriptArtifactId);
    const startedAt = performance.now();
    if (source === undefined) {
      await writer.send(
        failure(
          message.invocationId,
          startedAt,
          'request_invalid',
          'WOML_PROVIDER_ARTIFACT_UNKNOWN',
          'The immutable provider script artifact is not registered.',
          false
        )
      );
      return;
    }
    const worker = new Worker(workerPath(), { env: {} });
    const timer = setTimeout(async () => {
      if (!active.delete(message.invocationId)) return;
      await worker.terminate();
      await writer.send(
        failure(
          message.invocationId,
          startedAt,
          'timed_out',
          'WOML_PROVIDER_TIMED_OUT',
          'The custom provider exceeded its execution deadline.',
          true
        )
      );
    }, message.limits.timeoutMs);
    active.set(message.invocationId, { worker, startedAt, timer });
    worker.once('message', async (response: WorkerResponse) => {
      const invocation = active.get(message.invocationId);
      if (invocation === undefined) return;
      active.delete(message.invocationId);
      clearTimeout(invocation.timer);
      await worker.terminate();
      const completion = response.ok
        ? completed(message.invocationId, startedAt, {
            kind: 'succeeded',
            receipt: response.receipt ?? {},
          })
        : failure(
            message.invocationId,
            startedAt,
            response.kind ?? 'script_threw',
            response.kind === 'service_failed'
              ? 'WOML_PROVIDER_SERVICE_FAILED'
              : 'WOML_PROVIDER_SCRIPT_FAILED',
            response.message ?? 'The provider script failed.',
            response.retryable ?? false
          );
      if (Buffer.byteLength(JSON.stringify(completion), 'utf8') > message.limits.maxResultBytes) {
        await writer.send(
          failure(
            message.invocationId,
            startedAt,
            'result_too_large',
            'WOML_PROVIDER_RESULT_TOO_LARGE',
            'The custom provider receipt exceeds the configured result limit.',
            false
          )
        );
      } else {
        await writer.send(completion);
      }
    });
    worker.once('error', async () => {
      const invocation = active.get(message.invocationId);
      if (invocation === undefined) return;
      active.delete(message.invocationId);
      clearTimeout(invocation.timer);
      await writer.send(
        failure(
          message.invocationId,
          startedAt,
          'worker_crashed',
          'WOML_PROVIDER_WORKER_CRASHED',
          'The isolated provider worker stopped unexpectedly.',
          false
        )
      );
    });
    worker.postMessage({
      source,
      props: message.props,
      notification: message.notification,
      attempt: message.attempt,
    });
  };

  await writer.send({
    protocol: CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL,
    protocolVersion: CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
    messageType: 'ready',
    hostInstanceId: `custom_provider_host_${randomUUID()}`,
  });
  const reader = Bun.stdin.stream().getReader();
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    for (const raw of decoder.push(value)) {
      try {
        assertCustomProviderInbound(raw, new Set(artifacts.keys()));
      } catch (error) {
        const invocationId =
          typeof raw === 'object' && raw !== null && 'invocationId' in raw &&
          typeof raw.invocationId === 'string'
            ? raw.invocationId
            : undefined;
        if (invocationId === undefined) throw error;
        await writer.send(
          failure(
            invocationId,
            performance.now(),
            'request_invalid',
            'WOML_PROVIDER_REQUEST_INVALID',
            error instanceof Error ? error.message : 'Invalid provider request.',
            false
          )
        );
        continue;
      }
      if (raw.messageType === 'execute') {
        void execute(raw);
      } else {
        const invocation = active.get(raw.invocationId);
        if (invocation === undefined) continue;
        active.delete(raw.invocationId);
        clearTimeout(invocation.timer);
        await invocation.worker.terminate();
        await writer.send(
          failure(
            raw.invocationId,
            invocation.startedAt,
            'cancelled',
            'WOML_PROVIDER_CANCELLED',
            'The custom provider invocation was cancelled.',
            false
          )
        );
      }
    }
  }
  decoder.finish();
  while (active.size > 0) {
    await Bun.sleep(5);
  }
  await writer.drain();
}

if (import.meta.main) {
  await runCustomNotificationProviderHost().catch(error => {
    process.stderr.write(
      `WOML custom notification provider host failed: ${error instanceof Error ? error.message : String(error)}\n`
    );
    process.exitCode = 1;
  });
}
