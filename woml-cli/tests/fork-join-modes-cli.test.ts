import { describe, expect, test } from 'bun:test';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';
import { createSecretStore } from '../src/secrets';

const fixtureRoot = resolve(
  import.meta.dir,
  '../../woml/tests/fixtures/fork-branch'
);
const nativeCorePath = resolve(
  import.meta.dir,
  `../dist/woml-core.${process.platform}-${process.arch}.node`
);
const projectRoot = resolve(import.meta.dir, '../..');

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
    nativeCorePath,
  });
  return { exitCode, stdout, stderr };
}

describe('selected and non-blocking fork joins in the CLI', () => {
  test('a main-route condition waits for the selected join before workflow settlement', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-fork-main-choice-cli-'));
    try {
      const result = await invoke([
        'test',
        join(projectRoot, 'examples/manualForkWorkflow.woml'),
        '--state',
        join(directory, 'state.sqlite'),
      ]);
      expect(result.exitCode, result.stderr).toBe(0);
      expect(JSON.parse(result.stdout)).toEqual({
        orderId: 'order-42',
        inventory: 'items-reserved',
        payment: { paid: true, amount: 120 },
        fulfillment: { status: 'ready-to-ship', orderId: 'order-42' },
      });
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  test('selected join executes only after its named branches and publishes their outputs', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-selected-join-cli-'));
    try {
      const result = await invoke([
        'test',
        join(fixtureRoot, 'join-selected.woml'),
        '--state',
        join(directory, 'state.sqlite'),
      ]);
      expect(result.exitCode).toBe(0);
      expect(JSON.parse(result.stdout)).toEqual({
        instagram: { status: 'published' },
        facebook: { status: 'published' },
      });
      expect(result.stderr).not.toContain('UNSUPPORTED_FORK_EXECUTION');
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  test('non-blocking join returns the main result only after its branch remains owned and settles', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-none-join-cli-'));
    try {
      const result = await invoke([
        'test',
        join(fixtureRoot, 'join-none.woml'),
        '--state',
        join(directory, 'state.sqlite'),
      ]);
      expect(result.exitCode).toBe(0);
      expect(JSON.parse(result.stdout)).toEqual({ accepted: true });
      expect(result.stderr).not.toContain('UNSUPPORTED_FORK_EXECUTION');
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  test('maps a failing branch step back to its WOML source location', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-fork-error-cli-'));
    const workflowPath = join(directory, 'failing-branch.woml');
    await writeFile(
      workflowPath,
      `<woml>
<workflow id="fork-source-error" version="1.0.0">
  <triggers><manual id="start" /></triggers>
  <steps>
    <step id="prepare"><script>return { ready: true };</script></step>
    <fork id="distribution" join="all">
      <branch id="instagram">
        <step id="publishInstagram">
          <script>throw new Error("Instagram unavailable");</script>
        </step>
      </branch>
      <branch id="archive">
        <step id="archive"><script>return { archived: true };</script></step>
      </branch>
    </fork>
    <step id="finish"><script>return { done: true };</script></step>
  </steps>
</workflow>
</woml>`
    );
    try {
      const result = await invoke([
        'test',
        workflowPath,
        '--state',
        join(directory, 'state.sqlite'),
      ]);
      expect(result.exitCode).toBe(1);
      expect(result.stdout).toBe('');
      expect(result.stderr).toContain('WOML runtime error [WOML_SCRIPT_FAILED]');
      expect(result.stderr).toContain(`${workflowPath}:9:`);
      expect(result.stderr).toContain('step "publishInstagram"');
      expect(result.stderr).toContain('Instagram unavailable');
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });
});
