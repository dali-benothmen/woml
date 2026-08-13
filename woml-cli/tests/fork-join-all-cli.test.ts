import { describe, expect, test } from 'bun:test';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';
import { createSecretStore } from '../src/secrets';

const fixture = resolve(
  import.meta.dir,
  '../../woml/tests/fixtures/fork-branch/join-all.woml'
);
const nativeCorePath = resolve(
  import.meta.dir,
  `../dist/woml-core.${process.platform}-${process.arch}.node`
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
    nativeCorePath,
  });
  return { exitCode, stdout, stderr };
}

describe('join-all fork CLI execution', () => {
  test('check reports Rust support and test returns the reviewed main result', async () => {
    const checked = await invoke(['check', fixture]);
    expect(checked.exitCode).toBe(0);
    expect(checked.stderr).toBe('');
    expect(checked.stdout).toContain(
      'Model v13 all, selected, and non-blocking fork joins are executable through the durable Rust runtime'
    );

    const directory = await mkdtemp(join(tmpdir(), 'woml-fork-join-all-cli-'));
    try {
      const executed = await invoke([
        'test',
        fixture,
        '--state',
        join(directory, 'state.sqlite'),
      ]);
      if (executed.exitCode !== 0) {
        throw new Error(`fork/parallel execution failed:\n${executed.stderr}`);
      }
      expect(executed.exitCode).toBe(0);
      expect(JSON.parse(executed.stdout)).toEqual({
        instagram: { platform: 'instagram' },
        facebook: { platform: 'facebook' },
      });
      expect(executed.stderr).toContain('started under runtime policy');
      expect(executed.stderr).not.toContain('UNSUPPORTED_FORK_EXECUTION');
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  test('parallel work composes inside a branch before the fork joins', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-fork-parallel-cli-'));
    const workflowPath = join(directory, 'parallel-in-branch.woml');
    await writeFile(
      workflowPath,
      `<woml>
<workflow id="fork-parallel-composition" version="1.0.0">
  <triggers><manual id="start" /></triggers>
  <steps>
    <step id="prepare"><script>return { ready: true };</script></step>
    <fork id="distribution" join="all">
      <branch id="social">
        <parallel id="socialPosts">
          <step id="instagram"><script>return { sent: "instagram" };</script></step>
          <step id="facebook"><script>return { sent: "facebook" };</script></step>
        </parallel>
        <step id="socialDone"><script>return { done: true };</script></step>
      </branch>
      <branch id="archive">
        <step id="archivePost"><script>return { archived: true };</script></step>
      </branch>
    </fork>
    <step id="finish">
      <script>
        return {
          instagram: context.steps.instagram.sent,
          facebook: context.steps.facebook.sent,
          archived: context.steps.archivePost.archived
        };
      </script>
    </step>
  </steps>
</workflow>
</woml>`
    );
    try {
      const executed = await invoke([
        'test',
        workflowPath,
        '--state',
        join(directory, 'state.sqlite'),
      ]);
      expect(executed.exitCode).toBe(0);
      expect(JSON.parse(executed.stdout)).toEqual({
        instagram: 'instagram',
        facebook: 'facebook',
        archived: true,
      });
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });
});
