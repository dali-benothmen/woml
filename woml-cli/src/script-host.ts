#!/usr/bin/env bun

import { randomUUID } from 'node:crypto';

import { FrameDecoder, SerializedFrameWriter } from './script-host/framing';
import { ScriptHost } from './script-host/host';
import type {
  ReadyMessage,
  ScriptHostLimits,
  ScriptHostProtocolVersion,
} from './script-host/types';

function configuredLimit(name: string): number | undefined {
  const raw = process.env[name];
  if (raw === undefined || raw === '') return undefined;
  const value = Number(raw);
  if (!Number.isSafeInteger(value) || value < 1) {
    throw new Error(`${name} must be a positive safe integer when provided.`);
  }
  return value;
}

function limitsFromEnvironment(): ScriptHostLimits {
  return {
    maxContextBytes: configuredLimit('WOML_SCRIPT_HOST_MAX_CONTEXT_BYTES'),
    maxResultBytes: configuredLimit('WOML_SCRIPT_HOST_MAX_RESULT_BYTES'),
    maxFrameBytes: configuredLimit('WOML_SCRIPT_HOST_MAX_FRAME_BYTES'),
  };
}

function protocolVersionFromEnvironment(): ScriptHostProtocolVersion {
  const raw = process.env.WOML_SCRIPT_HOST_PROTOCOL_VERSION ?? '3';
  if (raw === '1' || raw === '2' || raw === '3' || raw === '4') {
    return Number(raw) as ScriptHostProtocolVersion;
  }
  throw new Error('WOML_SCRIPT_HOST_PROTOCOL_VERSION must be 1, 2, 3, or 4.');
}

async function writeStdout(frame: Uint8Array): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    process.stdout.write(frame, error => {
      if (error === null || error === undefined) resolve();
      else reject(error);
    });
  });
}

export async function runScriptHost(): Promise<void> {
  const limits = limitsFromEnvironment();
  const protocolVersion = protocolVersionFromEnvironment();
  const decoder = new FrameDecoder({ maxFrameBytes: limits.maxFrameBytes });
  const writer = new SerializedFrameWriter(writeStdout);
  const workerEntry = import.meta.url.endsWith('.ts')
    ? './script-host-worker.ts'
    : './script-host-worker.js';
  const host = new ScriptHost({
    workerUrl: new URL(workerEntry, import.meta.url),
    limits,
    protocolVersion,
    send: message => writer.send(message),
  });
  const ready: ReadyMessage = {
    protocol: 'woml.script-host',
    protocolVersion,
    messageType: 'ready',
    hostInstanceId: `host_${randomUUID()}`,
  };

  try {
    await writer.send(ready);
    const reader = Bun.stdin.stream().getReader();
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      for (const message of decoder.push(value)) host.accept(message);
    }
    decoder.finish();
    await host.drain();
    await writer.drain();
  } catch (error) {
    host.abort();
    const message =
      error instanceof Error
        ? `${error.name}: ${error.message}`
        : String(error);
    process.stderr.write(`WOML script host failed: ${message}\n`);
    process.exitCode = 1;
  }
}

if (import.meta.main) await runScriptHost();
