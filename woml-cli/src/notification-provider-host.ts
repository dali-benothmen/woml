#!/usr/bin/env bun

import { randomUUID } from 'node:crypto';

import { FrameDecoder, SerializedFrameWriter } from './script-host/framing';
import { createSecretStore } from './secrets';
import { FakeSlackTransport } from './notification-provider/fake-slack';
import { NotificationProviderHost } from './notification-provider/host';
import { RealSlackTransport } from './notification-provider/real-slack';
import type { SlackTransport } from './notification-provider/slack-transport';
import {
  NOTIFICATION_PROVIDER_MAX_FRAME_BYTES,
  NOTIFICATION_PROVIDER_PROTOCOL,
  NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
  type ApprovalDecision,
  type ReadyMessage,
} from './notification-provider/types';

function automaticDecision(): ApprovalDecision | undefined {
  const value = process.env.WOML_FAKE_SLACK_DECISION;
  if (value === undefined || value === '') return undefined;
  if (value === 'approved' || value === 'rejected') return value;
  throw new Error('WOML_FAKE_SLACK_DECISION must be approved or rejected.');
}

function automaticDelay(): number {
  const raw = process.env.WOML_FAKE_SLACK_ACTION_DELAY_MS ?? '0';
  const value = Number(raw);
  if (!Number.isSafeInteger(value) || value < 0 || value > 60_000) {
    throw new Error(
      'WOML_FAKE_SLACK_ACTION_DELAY_MS must be an integer from 0 through 60000.'
    );
  }
  return value;
}

function fakeDeliveryFailures(): number {
  const raw = process.env.WOML_FAKE_SLACK_DELIVERY_FAILURES ?? '0';
  const value = Number(raw);
  if (!Number.isSafeInteger(value) || value < 0 || value > 2) {
    throw new Error(
      'WOML_FAKE_SLACK_DELIVERY_FAILURES must be an integer from 0 through 2.'
    );
  }
  return value;
}

async function writeStdout(frame: Uint8Array): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    process.stdout.write(frame, error => {
      if (error === null || error === undefined) resolve();
      else reject(error);
    });
  });
}

export interface RunNotificationProviderHostOptions {
  readonly adapter?: 'real' | 'fake';
  readonly createTransport?: (
    emit: (message: Parameters<SerializedFrameWriter['send']>[0]) => Promise<void>
  ) => SlackTransport;
}

export async function runNotificationProviderHost(
  options: RunNotificationProviderHostOptions = {}
): Promise<void> {
  const writer = new SerializedFrameWriter(writeStdout);
  const decoder = new FrameDecoder({
    maxFrameBytes: NOTIFICATION_PROVIDER_MAX_FRAME_BYTES,
  });
  const emit = (message: Parameters<typeof writer.send>[0]) =>
    writer.send(message);
  const transport =
    options.createTransport?.(emit) ??
    (options.adapter === 'fake'
      ? new FakeSlackTransport({
          emit,
          automaticDecision: automaticDecision(),
          automaticActorId:
            process.env.WOML_FAKE_SLACK_ACTOR_ID ?? 'U12345678',
          automaticDelayMs: automaticDelay(),
          deliveryFailuresBeforeSuccess: fakeDeliveryFailures(),
        })
      : new RealSlackTransport({
          emit,
          log: message => process.stderr.write(`[woml] ${message}\n`),
        }));
  const host = new NotificationProviderHost({
    secretStore: createSecretStore(),
    transport,
    send: message => writer.send(message),
  });
  const ready: ReadyMessage = {
    protocol: NOTIFICATION_PROVIDER_PROTOCOL,
    protocolVersion: NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
    messageType: 'ready',
    hostInstanceId: `notification_host_${randomUUID()}`,
    providers: ['slack'],
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
    await host.close();
    await writer.drain();
  } catch (error) {
    await host.close().catch(() => {});
    const message = error instanceof Error ? error.message : String(error);
    process.stderr.write(`WOML notification provider host failed: ${message}\n`);
    process.exitCode = 1;
  }
}

if (import.meta.main) await runNotificationProviderHost({ adapter: 'real' });
