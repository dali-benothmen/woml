import { afterAll, describe, expect, test } from 'bun:test';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { nativePackageBinaryName } from '../src/native-platform';
import { installLocalReleaseCandidate } from './helpers/release-candidate';

const scriptPath = Bun.which('script');
const packagedTest = process.platform === 'win32' || scriptPath === null
  ? test.skip
  : test;
const directories: string[] = [];

afterAll(async () => {
  await Promise.all(directories.map(directory =>
    rm(directory, { recursive: true, force: true })
  ));
});

function shellQuote(value: string): string {
  return `'${value.replaceAll("'", `'\\''`)}'`;
}

async function waitUntil(
  predicate: () => boolean,
  description: string,
  output: () => string,
  timeoutMs = 20_000
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (!predicate()) {
    if (Date.now() >= deadline) {
      throw new Error(`Timed out waiting for ${description}:\n${output()}`);
    }
    await Bun.sleep(10);
  }
}

function invoke(executable: string, cwd: string, ...args: string[]) {
  return Bun.spawnSync([executable, ...args], {
    cwd,
    stdout: 'pipe',
    stderr: 'pipe',
  });
}

describe('clean installed terminal experience', () => {
  packagedTest('packs, installs, waits for Enter, executes through Rust/Bun, and exits cleanly', async () => {
    const root = await mkdtemp(join(tmpdir(), 'woml-terminal-release-'));
    directories.push(root);
    const consumer = join(root, 'consumer');
    const cache = join(root, 'cache');
    await Promise.all([consumer, cache].map(path =>
      mkdir(path, { recursive: true })
    ));

    await writeFile(
      join(consumer, 'package.json'),
      JSON.stringify({ name: 'woml-terminal-clean-consumer', private: true })
    );
    const candidate = await installLocalReleaseCandidate(consumer, { cache });

    const executable = join(consumer, 'node_modules/.bin/woml');
    const installedPackage = join(consumer, 'node_modules/woml');
    expect(await Bun.file(join(installedPackage, 'dist/cli.js')).exists()).toBe(true);
    expect(
      await Bun.file(
        join(
          consumer,
          'node_modules',
          ...candidate.nativePackage.split('/'),
          nativePackageBinaryName(candidate.target),
        ),
      ).exists(),
    ).toBe(true);
    expect(
      await Bun.file(join(installedPackage, 'dist/woml-core.node')).exists(),
    ).toBe(false);
    expect(await Bun.file(join(installedPackage, 'src/cli.ts')).exists()).toBe(false);

    const version = invoke(executable, consumer, '--version');
    expect(version.exitCode, version.stderr.toString()).toBe(0);
    expect(version.stdout.toString().trim()).toMatch(/^woml \d+\.\d+\.\d+$/);

    const workflowPath = join(consumer, 'manual.woml');
    const statePath = join(consumer, 'state.sqlite');
    await writeFile(workflowPath, `<woml>
  <workflow id="installed-manual" name="Installed manual" description="Clean package acceptance." version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps>
      <step id="prepare" name="Prepare result"><script>return { value: 21 };</script></step>
      <step id="finish" name="Finish result"><script>return { value: context.steps.prepare.value * 2 };</script></step>
    </steps>
  </workflow>
</woml>`);

    const command = [
      shellQuote(executable),
      'run',
      shellQuote(workflowPath),
      '--state',
      shellQuote(statePath),
      '--color=always',
    ].join(' ');
    const child = Bun.spawn(
      [scriptPath!, '-qefc', command, '/dev/null'],
      { cwd: consumer, stdin: 'pipe', stdout: 'pipe', stderr: 'pipe' }
    );
    let output = '';
    const collect = async (stream: ReadableStream<Uint8Array>): Promise<void> => {
      const reader = stream.getReader();
      const decoder = new TextDecoder();
      for (;;) {
        const chunk = await reader.read();
        if (chunk.done) break;
        output += decoder.decode(chunk.value, { stream: true });
      }
      output += decoder.decode();
    };
    const stdoutDone = collect(child.stdout);
    const stderrDone = collect(child.stderr);

    try {
      await waitUntil(
        () => output.includes('Ready') && output.includes('Press Enter to run'),
        'the installed manual prompt',
        () => output
      );
      const before = invoke(executable, consumer, 'list', '--state', statePath, '--json');
      expect(before.exitCode, before.stderr.toString()).toBe(0);
      expect(JSON.parse(before.stdout.toString()).runs).toHaveLength(0);

      child.stdin.write('\n');
      await child.stdin.flush();
      await waitUntil(
        () => output.includes('RUN COMPLETED') && output.includes('Finish result'),
        'the installed durable run presentation',
        () => output
      );
      expect(output).toContain('\u001b[');
      expect(output).toContain('{ value: 42 }');

      const after = invoke(executable, consumer, 'list', '--state', statePath, '--json');
      expect(after.exitCode, after.stderr.toString()).toBe(0);
      expect(JSON.parse(after.stdout.toString()).runs).toHaveLength(1);

      child.stdin.write('\x03');
      await child.stdin.flush();
      expect(await child.exited).toBe(0);
      await Promise.all([stdoutDone, stderrDone]);
    } finally {
      if (child.exitCode === null) child.kill('SIGKILL');
      await child.exited;
    }
  }, 60_000);
});
