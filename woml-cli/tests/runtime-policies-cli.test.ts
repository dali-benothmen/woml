import { describe, expect, test } from 'bun:test';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { compileWoml, parseWoml } from '@woml/compiler';

import { formatExecutionProgress, runCli, type CliIo } from '../src/cli';
import {
  executeWorkflowWithRustDurable,
  type ExecutionProgressV1,
} from '../src/rust-executor';
import { createSecretStore } from '../src/secrets';

const fixtureRoot = resolve(
  import.meta.dir,
  '../../woml/tests/fixtures/runtime-policies'
);

async function invoke(args: readonly string[]) {
  let stdout = '';
  let stderr = '';
  const io: CliIo = {
    stdout: value => {
      stdout += value;
    },
    stderr: value => {
      stderr += value;
    },
  };
  const exitCode = await runCli(args, io, {
    createSecretStore: () => createSecretStore(),
    readSecret: async () => '',
    nativeCorePath: resolve(
      import.meta.dir,
      `../dist/woml-core.${process.platform}-${process.arch}.node`
    ),
  });
  return { exitCode, stdout, stderr };
}

describe('Integrated runtime-policy CLI boundary', () => {
  test('woml check accepts config and reports the executable policy set', async () => {
    const result = await invoke([
      'check',
      resolve(fixtureRoot, 'runtime-policy.woml'),
    ]);
    expect(result.exitCode).toBe(0);
    expect(result.stderr).toBe('');
    expect(result.stdout).toContain('WOML check passed');
    expect(result.stdout).toContain('Model v12 concurrency');
    expect(result.stdout).toContain('rolling-window rate limits');
    expect(result.stdout).toContain('workflow timeouts are executable');
  });

  test('woml check --json exposes Definition Package v7 and Model v12 for modules', async () => {
    const result = await invoke([
      'check',
      resolve(fixtureRoot, 'runtime-policy-module.woml'),
      '--json',
    ]);
    expect(result.exitCode).toBe(0);
    expect(result.stderr).toBe('');
    expect(JSON.parse(result.stdout)).toMatchObject({
      schemaVersion: 7,
      profile: 'woml.definition-package/v7',
      runtimeReady: false,
      workflow: {
        model: {
          schemaVersion: 12,
          runtimePolicy: { profileVersion: 1, concurrency: 2 },
        },
      },
    });
  });

  test('woml test executes rate-limit and timeout policy fields', async () => {
    const result = await invoke([
      'test',
      resolve(fixtureRoot, 'runtime-policy.woml'),
    ]);
    expect(result.exitCode).toBe(0);
    expect(result.stderr).not.toContain(
      'WOML_RUNTIME_POLICY_RUNTIME_UNAVAILABLE'
    );
    expect(JSON.parse(result.stdout)).toEqual({ ok: true });
  });

  test('runs a Model v12 workflow with a local module', async () => {
    const result = await invoke([
      'test',
      resolve(fixtureRoot, 'runtime-policy-module.woml'),
    ]);
    expect(result.exitCode).toBe(0);
    expect(result.stderr).toContain('WOML modules ready: services.text.');
    expect(result.stderr).toContain('started under runtime policy');
    expect(JSON.parse(result.stdout)).toEqual({ value: 'hello' });
  });

  test('list/get JSON use policy-aware v2/v3 operator contracts', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-rp6-cli-'));
    const statePath = join(directory, 'state.sqlite');
    try {
      const filePath = resolve(fixtureRoot, 'runtime-policy.woml');
      const source = await Bun.file(filePath).text();
      const workflow = compileWoml(parseWoml(source, { file: filePath }));
      const progress: ExecutionProgressV1[] = [];
      const execution = await executeWorkflowWithRustDurable(
        workflow,
        statePath,
        {
          nativeCorePath: resolve(
            import.meta.dir,
            `../dist/woml-core.${process.platform}-${process.arch}.node`
          ),
          onProgress: message => progress.push(message),
        }
      );
      expect(progress.map(formatExecutionProgress)).toContain(
        `Run ${execution.runId} started under runtime policy.`
      );

      const list = await invoke(['list', '--state', statePath, '--json']);
      expect(list.exitCode).toBe(0);
      expect(JSON.parse(list.stdout)).toMatchObject({
        profile: 'woml.run-list/v2',
        runs: [
          {
            runId: execution.runId,
            workflowId: 'policy-demo',
            status: 'succeeded',
            queue: 'orders',
          },
        ],
      });

      const get = await invoke([
        'get',
        execution.runId,
        '--state',
        statePath,
        '--json',
      ]);
      expect(get.exitCode).toBe(0);
      expect(JSON.parse(get.stdout)).toMatchObject({
        profile: 'woml.run-inspection/v3',
        runId: execution.runId,
        policy: { queue: 'orders' },
      });
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });
});
