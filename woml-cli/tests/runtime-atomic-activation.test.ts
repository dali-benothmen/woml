import { afterEach, describe, expect, test } from 'bun:test';
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
import { sourceSnapshotDigest } from '../src/compiled-workflow-cache';
import { SharedSlackTransport } from '../src/notification-provider';
import {
  activateWebhookRuntimeWithRust,
  compiledDefinitionHash,
  listRunsWithRust,
  startWebhookRuntimeWithRust,
  stopWebhookRuntimeWithRust,
  submitExternalSignalWithRust,
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
const durableWaitModelPath = resolve(
  packageRoot,
  '../woml/tests/fixtures/durable-waits/model.v17.reviewed.json'
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
  test('uses order-independent activation identities and rejects meaningful source changes', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-pro2-source-'));
    temporaryDirectories.push(directory);
    const path = join(directory, 'workflow.woml');
    const canonicalSource = '<woml>\n  <workflow id="stable" />\n</woml>\n';
    await Bun.write(path, canonicalSource.replaceAll('\n', '\r\n'));
    const digest = sourceSnapshotDigest(path, canonicalSource);
    const sourceSnapshot = [{ path, digest }];
    await expect(
      assertStableSourceSnapshot([{ sourceSnapshot }])
    ).resolves.toBeUndefined();

    await Bun.write(path, canonicalSource);
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

  test('submits an external signal through the staged native runtime boundary', async () => {
    if (!existsSync(nativeCorePath)) return;
    const directory = await mkdtemp(join(tmpdir(), 'woml-dw5-native-signal-'));
    temporaryDirectories.push(directory);
    const statePath = join(directory, 'state.sqlite');
    const model = (await Bun.file(durableWaitModelPath).json()) as Record<
      string,
      any
    >;
    model.graph.waits = [
      {
        waitId: 'pause',
        nodeId: 'pause',
        metadata: { name: 'Wait for job' },
        kind: 'signal',
        signals: [
          {
            signal: 'job.completed',
            schema: {
              type: 'object',
              properties: { status: { type: 'string' } },
              required: ['status'],
              additionalProperties: false,
            },
          },
        ],
        correlation: { kind: 'literal', value: 'job-42' },
        timeoutMs: 60_000,
        onTimeout: 'fail',
      },
    ];
    const workflow = model as unknown as CompiledWorkflowDefinition;
    const handle = await startWebhookRuntimeWithRust(
      [
        {
          workflow,
          definitionHash: compiledDefinitionHash(workflow),
          resolvedSecrets: {},
        },
      ],
      statePath,
      {
        nativeCorePath,
        scriptHostPath,
        host: '127.0.0.1',
        port: 0,
        startupManualTriggers: { 'durable-wait-model': 'start' },
      }
    );
    try {
      const deadline = Date.now() + 5_000;
      let waitingRunId: string | undefined;
      while (Date.now() < deadline) {
        waitingRunId = listRunsWithRust(
          statePath,
          { limit: 10 },
          { nativeCorePath }
        ).runs.find(run => run.status === 'waiting')?.runId;
        if (waitingRunId !== undefined) break;
        await Bun.sleep(10);
      }
      expect(waitingRunId).toBeDefined();
      const outcome = await submitExternalSignalWithRust(
        handle.runtimeId,
        {
          profile: 'woml.external-signal-admission/v1',
          signal: 'job.completed',
          runId: waitingRunId!,
          waitId: 'pause',
          correlation: 'job-42',
          idempotencyKey: 'native-callback-1',
          payload: { status: 'passed' },
          receivedAt: new Date().toISOString(),
          authenticationClass: 'local_control',
        },
        { nativeCorePath }
      );
      expect(outcome).toMatchObject({
        profile: 'woml.signal-result/v1',
        status: 'accepted',
        resolution: 'resolved',
        runId: waitingRunId,
      });
      while (Date.now() < deadline) {
        const settled = listRunsWithRust(
          statePath,
          { limit: 10 },
          { nativeCorePath }
        ).runs.find(run => run.runId === waitingRunId);
        if (settled?.status === 'succeeded') break;
        await Bun.sleep(10);
      }
      expect(
        listRunsWithRust(statePath, { limit: 10 }, { nativeCorePath }).runs.find(
          run => run.runId === waitingRunId
        )?.status
      ).toBe('succeeded');
    } finally {
      await stopWebhookRuntimeWithRust(handle.runtimeId, { nativeCorePath });
    }
  }, 15_000);
});
