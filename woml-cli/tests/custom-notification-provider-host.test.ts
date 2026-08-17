import { afterEach, describe, expect, test } from 'bun:test';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';

import {
  CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL,
  CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
  type CustomProviderExecuteMessage,
  type CustomProviderOutbound,
} from '../src/custom-notification-provider';
import { encodeFrame, FrameDecoder } from '../src/script-host/framing';

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(
    temporaryDirectories.splice(0).map(path => rm(path, { recursive: true }))
  );
});

async function runHost(
  source: string,
  messages: readonly CustomProviderExecuteMessage[]
): Promise<readonly CustomProviderOutbound[]> {
  return (await runHostWithLogs(source, messages)).messages;
}

async function runHostWithLogs(
  source: string,
  messages: readonly CustomProviderExecuteMessage[]
): Promise<{
  readonly messages: readonly CustomProviderOutbound[];
  readonly stderr: string;
}> {
  const directory = await mkdtemp(resolve(tmpdir(), 'woml-provider-host-'));
  temporaryDirectories.push(directory);
  const manifest = resolve(directory, 'artifacts.json');
  await writeFile(
    manifest,
    JSON.stringify({
      artifacts: [{ scriptArtifactId: 'provider_fixture', source }],
    })
  );
  const child = Bun.spawn(
    [
      process.execPath,
      resolve(import.meta.dir, '../src/custom-notification-provider-host.ts'),
      manifest,
    ],
    {
      stdin: Buffer.concat(messages.map(encodeFrame)),
      stdout: 'pipe',
      stderr: 'pipe',
    }
  );
  const [stdout, stderr, exitCode] = await Promise.all([
    new Response(child.stdout).arrayBuffer(),
    new Response(child.stderr).text(),
    child.exited,
  ]);
  expect(exitCode, stderr).toBe(0);
  const decoder = new FrameDecoder();
  const decoded = decoder.push(new Uint8Array(stdout));
  decoder.finish();
  return { messages: decoded as CustomProviderOutbound[], stderr };
}

function execute(
  invocationId: string,
  overrides: Partial<CustomProviderExecuteMessage> = {}
): CustomProviderExecuteMessage {
  return {
    protocol: CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL,
    protocolVersion: CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
    messageType: 'execute',
    invocationId,
    definitionDigest: `sha256:${'a'.repeat(64)}`,
    scriptArtifactId: 'provider_fixture',
    props: { token: 'super-secret-value' },
    notification: {
      kind: 'approval',
      message: 'Approve this?',
      deliveryId: 'approval:notify:0',
      idempotencyKey: `sha256:${'b'.repeat(64)}`,
      actions: {
        approve: {
          url: 'http://127.0.0.1:7331/api/v1/notification-approvals/ncap_secret/approved',
        },
        reject: {
          url: 'http://127.0.0.1:7331/api/v1/notification-approvals/ncap_secret/rejected',
        },
      },
    },
    attempt: { number: 1, max: 3 },
    limits: { timeoutMs: 2_000, maxResultBytes: 16_384 },
    ...overrides,
  };
}

describe('custom notification provider host', () => {
  test('executes provider scripts with props and capability URLs', async () => {
    const messages = await runHost(
      `
        if (props.token !== 'super-secret-value') throw new Error('missing prop');
        if (!notification.actions.approve.url.endsWith('/approved')) throw new Error('missing action');
        return { messageId: 'telegram-42' };
      `,
      [execute('provider-success')]
    );
    expect(messages).toHaveLength(2);
    expect(messages[1]).toMatchObject({
      messageType: 'completed',
      invocationId: 'provider-success',
      outcome: { kind: 'succeeded', receipt: { messageId: 'telegram-42' } },
    });
  });

  test('redacts secret props and decision capabilities from failures', async () => {
    const messages = await runHost(
      `throw new Error(props.token + ' ' + notification.actions.approve.url);`,
      [execute('provider-redaction')]
    );
    const serialized = JSON.stringify(messages[1]);
    expect(messages[1]).toMatchObject({
      outcome: { kind: 'failed', error: { kind: 'script_threw' } },
    });
    expect(serialized).not.toContain('super-secret-value');
    expect(serialized).not.toContain('ncap_secret');
  });

  test('keeps colored provider logs off the framed protocol and redacts capabilities', async () => {
    const result = await runHostWithLogs(
      `
        console.log('\u001b[32;1m[SUCCESS]', notification.message, '\u001b[0m');
        console.error(props.token, notification.actions.approve.url);
        return { messageId: 'console-42' };
      `,
      [execute('provider-console')]
    );
    expect(result.messages).toHaveLength(2);
    expect(result.messages[1]).toMatchObject({
      messageType: 'completed',
      invocationId: 'provider-console',
      outcome: { kind: 'succeeded', receipt: { messageId: 'console-42' } },
    });
    expect(result.stderr).toContain('\u001b[32;1m[SUCCESS]');
    expect(result.stderr).toContain('Approve this?');
    expect(result.stderr).not.toContain('super-secret-value');
    expect(result.stderr).not.toContain('ncap_secret');
  });

  test('allows multiplexed invocations to complete out of order', async () => {
    const messages = await runHost(
      `
        await new Promise(resolve => setTimeout(resolve, Number(props.delay)));
        return { messageId: props.name };
      `,
      [
        execute('provider-slow', { props: { delay: '80', name: 'slow' } }),
        execute('provider-fast', { props: { delay: '1', name: 'fast' } }),
      ]
    );
    expect(
      messages.flatMap(message =>
        message.messageType === 'completed' ? [message.invocationId] : []
      )
    ).toEqual(['provider-fast', 'provider-slow']);
  });
});
