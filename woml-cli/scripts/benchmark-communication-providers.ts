#!/usr/bin/env bun

import { resolve } from 'node:path';

import { SharedDiscordTransport } from '../src/discord';
import { NotificationProviderHost } from '../src/notification-provider/host';
import { FakeSlackTransport } from '../src/notification-provider/fake-slack';
import { SlackNotificationAdapter } from '../src/notification-provider/slack-adapter';
import type { DeliverMessage, NotificationProviderOutbound } from '../src/notification-provider/types';
import type { SecretMetadata, SecretStore } from '../src/secrets';
import { SharedTelegramTransport } from '../src/telegram';
import { SharedWhatsAppTransport } from '../src/whatsapp';

const packageRoot = resolve(import.meta.dir, '..');

class MemorySecrets implements SecretStore {
  readonly provider = 'environment' as const;
  constructor(readonly values: Readonly<Record<string, string>>) {}
  async get(name: string): Promise<string | undefined> { return this.values[name]; }
  async has(name: string): Promise<boolean> { return this.values[name] !== undefined; }
  async list(): Promise<readonly SecretMetadata[]> {
    return Object.keys(this.values).map(name => ({ name, provider: this.provider }));
  }
  async set(): Promise<void> { throw new Error('read only'); }
  async delete(): Promise<boolean> { return false; }
}

function response(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function measureTest(file: string, pattern: string): number {
  const started = performance.now();
  const result = Bun.spawnSync(
    [Bun.which('bun')!, 'test', file, '--test-name-pattern', pattern],
    { cwd: packageRoot, stdout: 'pipe', stderr: 'pipe' }
  );
  if (result.exitCode !== 0) {
    throw new Error(`${file} failed:\n${result.stderr.toString()}`);
  }
  return performance.now() - started;
}

async function idleConnections(): Promise<{
  averageMs: number;
  memoryPerCredentialBytes: number;
}> {
  let identity = 1000;
  const fetch: typeof globalThis.fetch = async (input, init) => {
    const url = String(input);
    if (url.includes('/getMe')) {
      identity += 1;
      return response({ ok: true, result: { id: identity, is_bot: true, username: `bot${identity}` } });
    }
    if (url.includes('/getUpdates')) {
      return await new Promise<Response>((_resolve, reject) => {
        init?.signal?.addEventListener('abort', () => reject(new DOMException('Aborted', 'AbortError')), { once: true });
      });
    }
    throw new Error(`Unexpected idle benchmark request: ${url}`);
  };
  const transport = new SharedTelegramTransport({ fetch, retryDelayMs: 1 });
  const credentialCount = 32;
  Bun.gc(true);
  const before = process.memoryUsage().rss;
  const started = performance.now();
  for (let index = 0; index < credentialCount; index += 1) {
    await transport.ensurePolling(`TOKEN_${index}`, `100000:${'a'.repeat(35)}${index}`);
  }
  const elapsed = performance.now() - started;
  const after = process.memoryUsage().rss;
  await transport.close();
  return {
    averageMs: elapsed / credentialCount,
    memoryPerCredentialBytes: Math.max(0, after - before) / credentialCount,
  };
}

async function outbound(iterations: number): Promise<{
  averageMs: number;
  concurrentProvidersMs: number;
}> {
  let telegramId = 0;
  let discordId = 300000000000000000;
  let whatsappId = 0;
  const telegram = new SharedTelegramTransport({
    fetch: async () => response({
      ok: true,
      result: { message_id: ++telegramId, chat: { id: 123456789 } },
    }),
  });
  const discord = new SharedDiscordTransport({
    fetch: async () => response({
      id: String(++discordId),
      channel_id: '200000000000000001',
    }),
  });
  const whatsapp = new SharedWhatsAppTransport({
    fetch: async () => response({ messages: [{ id: `wamid.benchmark-${++whatsappId}` }] }),
  });
  const one = async (index: number) => {
    await telegram.sendMessage({
      botToken: `100000:${'a'.repeat(35)}`,
      accountId: '100000',
      conversationId: '123456789',
      text: `telegram-${index}`,
    });
    await discord.sendMessage({
      botToken: 'discord-benchmark-token',
      accountId: '200000000000000000',
      conversationId: '200000000000000001',
      text: `discord-${index}`,
    });
    await whatsapp.sendTemplate({
      accessToken: 'whatsapp-benchmark-token',
      phoneNumberId: '123456789012345',
      conversationId: '15551234567',
      templateName: 'woml_benchmark',
      language: 'en_US',
      parameters: [`whatsapp-${index}`],
    });
  };
  const sequentialStarted = performance.now();
  for (let index = 0; index < iterations; index += 1) await one(index);
  const averageMs = (performance.now() - sequentialStarted) / (iterations * 3);
  const concurrentStarted = performance.now();
  await Promise.all(Array.from({ length: iterations }, (_, index) => one(index)));
  const concurrentProvidersMs = performance.now() - concurrentStarted;
  await Promise.all([telegram.close(), discord.close(), whatsapp.close()]);
  return { averageMs, concurrentProvidersMs };
}

async function callbackResolution(iterations: number): Promise<number> {
  const sent: NotificationProviderOutbound[] = [];
  const transport = new FakeSlackTransport({ emit: async message => { sent.push(message); } });
  const host = new NotificationProviderHost({
    secretStore: new MemorySecrets({
      SLACK_BOT_TOKEN: 'xoxb-benchmark-token',
      SLACK_APP_TOKEN: 'xapp-benchmark-token',
    }),
    adapters: [new SlackNotificationAdapter(transport)],
    send: async message => { sent.push(message); },
  });
  const started = performance.now();
  for (let index = 0; index < iterations; index += 1) {
    const invocation: DeliverMessage = {
      protocol: 'woml.notification-provider-host',
      protocolVersion: 1,
      messageType: 'deliver',
      invocationId: `benchmark-${index}`,
      runId: `run_benchmark_${index}`,
      approvalId: 'review',
      requestId: `aprreq_benchmark_${index}`,
      deliveryId: `review:notify:0:channel:${index}`,
      provider: 'slack',
      destination: '#benchmark',
      idempotencyKey: `sha256:${index.toString(16).padStart(64, '0')}`,
      credentials: {
        botToken: { kind: 'secretReference', name: 'SLACK_BOT_TOKEN' },
        appToken: { kind: 'secretReference', name: 'SLACK_APP_TOKEN' },
      },
      decisionCapability: `ncap_${'1'.repeat(32)}.${index.toString(16).padStart(64, '0')}`,
      message: { workflowId: 'benchmark', approvalName: 'Review' },
    };
    host.accept(invocation);
  }
  await host.drain();
  for (let index = 0; index < iterations; index += 1) {
    await transport.click(`review:notify:0:channel:${index}`, 'approved', 'benchmark-user');
  }
  const averageMs = (performance.now() - started) / iterations;
  await host.close();
  if (sent.length < iterations * 2) throw new Error('Callback benchmark did not settle every delivery.');
  return averageMs;
}

const idle = await idleConnections();
const send = await outbound(25);
const callbackResolutionAverageMs = await callbackResolution(100);
const inboundAdmissionConformanceMs = measureTest(
  'tests/telegram-runtime.test.ts',
  'advances the polling offset only after Rust durably accepts a message'
);
const reconnectConformanceMs = measureTest(
  'tests/discord-runtime.test.ts',
  'resumes the shared Gateway session after a recoverable disconnect'
);

const measurements = {
  profile: 'woml.communication-provider-benchmark/v1',
  idleConnectionAverageMs: idle.averageMs,
  memoryPerCredentialIdentityBytes: idle.memoryPerCredentialBytes,
  inboundAdmissionConformanceMs,
  outboundSendAverageMs: send.averageMs,
  callbackResolutionAverageMs,
  reconnectConformanceMs,
  concurrentProvidersMs: send.concurrentProvidersMs,
};
const budgets = {
  idleConnectionAverageMs: 50,
  memoryPerCredentialIdentityBytes: 2 * 1024 * 1024,
  inboundAdmissionConformanceMs: 5_000,
  outboundSendAverageMs: 10,
  callbackResolutionAverageMs: 10,
  reconnectConformanceMs: 5_000,
  concurrentProvidersMs: 2_000,
};
for (const [name, maximum] of Object.entries(budgets)) {
  const observed = measurements[name as keyof typeof budgets] as number;
  if (!Number.isFinite(observed) || observed > maximum) {
    throw new Error(`${name} exceeded: ${observed.toFixed(2)} > ${maximum}.`);
  }
}
console.log(JSON.stringify({ ...measurements, budgets }, null, 2));
