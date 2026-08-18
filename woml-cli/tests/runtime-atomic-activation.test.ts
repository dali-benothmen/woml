import { afterEach, describe, expect, test } from 'bun:test';
import { createHash } from 'node:crypto';
import { existsSync } from 'node:fs';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import type { CompiledWorkflowDefinition } from '@woml/compiler';
import {
  activationIdentity,
  assertStableSourceSnapshot,
  runCli,
  type CliIo,
} from '../src/cli';
import { SharedSlackTransport } from '../src/notification-provider';
import {
  activateWebhookRuntimeWithRust,
  compiledDefinitionHash,
  listRunsWithRust,
  startWebhookRuntimeWithRust,
  stopWebhookRuntimeWithRust,
} from '../src/rust-executor';
import type { SecretMetadata, SecretStore } from '../src/secrets';

const packageRoot = resolve(import.meta.dir, '..');
const nativeCorePath = resolve(
  packageRoot,
  'dist',
  `woml-core.${process.platform}-${process.arch}.node`
);
const scriptHostPath = resolve(packageRoot, 'src/script-host.ts');
const modelPath = resolve(
  packageRoot,
  '../woml/tests/fixtures/triggers-webhook.compiled.v7.json'
);
const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(
    temporaryDirectories
      .splice(0)
      .map(path => rm(path, { recursive: true, force: true }))
  );
});

describe('Atomic Rust activation gate', () => {
  test('uses order-independent activation identities and rejects changed source bytes', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-pro2-source-'));
    temporaryDirectories.push(directory);
    const path = join(directory, 'workflow.woml');
    await Bun.write(path, '<woml />');
    const digest = `sha256:${createHash('sha256').update('<woml />').digest('hex')}`;
    const sourceSnapshot = [{ path, digest }];
    await expect(
      assertStableSourceSnapshot([{ sourceSnapshot }])
    ).resolves.toBeUndefined();

    const first = {
      workflow: { workflowId: 'first' },
      definitionHash: `sha256:${'a'.repeat(64)}`,
      runtimeModules: [],
    };
    const second = {
      workflow: { workflowId: 'second' },
      definitionHash: `sha256:${'b'.repeat(64)}`,
      runtimeModules: [],
    };
    expect(activationIdentity([first, second] as never)).toBe(
      activationIdentity([second, first] as never)
    );

    await Bun.write(path, '<woml changed="true" />');
    await expect(
      assertStableSourceSnapshot([{ sourceSnapshot }])
    ).rejects.toMatchObject({ code: 'WOML_SOURCE_CHANGED_DURING_ACTIVATION' });
  });

  test('binds prepared listeners closed, pins the definition, then admits only after activation', async () => {
    if (!existsSync(nativeCorePath)) return;
    const directory = await mkdtemp(join(tmpdir(), 'woml-pro2-gate-'));
    temporaryDirectories.push(directory);
    const workflow = (await Bun.file(
      modelPath
    ).json()) as CompiledWorkflowDefinition;
    const handle = await startWebhookRuntimeWithRust(
      [
        {
          workflow,
          definitionHash: compiledDefinitionHash(workflow),
          resolvedSecrets: {
            ORDER_WEBHOOK_TOKEN: 'pro2-test-token',
          },
        },
      ],
      join(directory, 'state.sqlite'),
      {
        nativeCorePath,
        scriptHostPath,
        host: '127.0.0.1',
        port: 0,
        startSuspended: true,
      }
    );

    try {
      const endpoint = `http://${handle.host}:${handle.port}/webhooks/orders`;
      const before = await fetch(endpoint, {
        method: 'POST',
        headers: {
          authorization: 'Bearer pro2-test-token',
          'content-type': 'application/json',
        },
        body: JSON.stringify({ orderId: 'before-ready' }),
      });
      expect(before.status).toBe(503);
      expect(await before.json()).toEqual({
        error: {
          code: 'WOML_RUNTIME_NOT_READY',
          message:
            'The WOML deployment is still activating; retry this request.',
        },
      });

      await activateWebhookRuntimeWithRust(handle.runtimeId, {
        nativeCorePath,
      });
      const after = await fetch(endpoint, {
        method: 'POST',
        headers: {
          authorization: 'Bearer pro2-test-token',
          'content-type': 'application/json',
        },
        body: JSON.stringify({ orderId: 'after-ready' }),
      });
      expect(after.status).toBe(202);
      expect(await after.json()).toMatchObject({ status: 'accepted' });
    } finally {
      await stopWebhookRuntimeWithRust(handle.runtimeId, { nativeCorePath });
    }
  }, 60_000);

  test('rolls back a prepared Rust runtime when a required provider fails readiness', async () => {
    if (!existsSync(nativeCorePath)) return;
    const directory = await mkdtemp(join(tmpdir(), 'woml-pro2-provider-'));
    temporaryDirectories.push(directory);
    const workflowPath = join(directory, 'slack.woml');
    const statePath = join(directory, 'state.sqlite');
    await Bun.write(
      workflowPath,
      `<woml><workflow id="provider-failure"><triggers><slack id="message" events="app-mention" channels="woml-testing" bot-token="{{secrets.SLACK_BOT_TOKEN}}" app-token="{{secrets.SLACK_APP_TOKEN}}" /></triggers><steps><step id="done"><script>return context.payload;</script></step></steps></workflow></woml>`
    );
    const values: Readonly<Record<string, string>> = {
      SLACK_BOT_TOKEN: 'xoxb-pro2-test',
      SLACK_APP_TOKEN: 'xapp-pro2-test',
    };
    const store: SecretStore = {
      provider: 'environment',
      get: async name => values[name],
      has: async name => values[name] !== undefined,
      list: async (): Promise<readonly SecretMetadata[]> =>
        Object.keys(values).map(name => ({ name, provider: 'environment' })),
      set: async () => {
        throw new Error('read only');
      },
      delete: async () => false,
    };
    let stdout = '';
    let stderr = '';
    let waited = false;
    const io: CliIo = {
      stdout: value => {
        stdout += value;
      },
      stderr: value => {
        stderr += value;
      },
    };
    const exitCode = await runCli(
      ['run', workflowPath, '--state', statePath],
      io,
      {
        createSecretStore: () => store,
        readSecret: async name => values[name] ?? '',
        nativeCorePath,
        waitForShutdown: async () => {
          waited = true;
        },
        createSlackTransport: options =>
          new SharedSlackTransport({
            ...options,
            fetch: (async () =>
              new Response(
                JSON.stringify({ ok: false, error: 'invalid_auth' })
              )) as unknown as typeof fetch,
          }),
      }
    );
    expect(exitCode).toBe(1);
    expect(stdout).toBe('');
    expect(stderr).toContain('WOML_SLACK_TRIGGER_UNAVAILABLE');
    expect(stderr).not.toContain('WOML automation is active');
    expect(waited).toBe(false);
    expect(
      listRunsWithRust(statePath, {}, { nativeCorePath }).runs
    ).toHaveLength(0);
  }, 15_000);
});
