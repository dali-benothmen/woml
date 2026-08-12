import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { mkdir, mkdtemp, readdir, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const packageRoot = resolve(import.meta.dir, '..');
let temporaryDirectory: string;

const counterWorkflow = `<woml>
<workflow id="ds5-clean-counter" name="Clean counter" version="1.0.0">
  <config concurrency="4" />
  <triggers><manual id="start" /></triggers>
  <steps><step id="count"><script>
    const counter = await services.state.increment('private-counter-key', 1, {
      name: 'count-clean-run'
    });
    return { count: counter.value, version: counter.version };
  </script></step></steps>
</workflow>
</woml>`;

const conversationWorkflow = `<woml>
<workflow id="ds5-clean-conversation" name="Clean conversation" version="1.0.0">
  <triggers><manual id="start" /></triggers>
  <steps><step id="remember"><script>
    const previous = await services.state.get('private-conversation-key');
    const turns = previous.found ? previous.value.turns + 1 : 1;
    const saved = await services.state.set(
      'private-conversation-key',
      { turns },
      {
        name: 'remember-clean-conversation',
        ifVersion: previous.found ? previous.version : 0
      }
    );
    return { turns, version: saved.version };
  </script></step></steps>
</workflow>
</woml>`;

beforeAll(async () => {
  temporaryDirectory = await mkdtemp(join(tmpdir(), 'woml-ds5-package-'));
});

afterAll(async () => {
  if (temporaryDirectory !== undefined) {
    await rm(temporaryDirectory, { recursive: true, force: true });
  }
});

describe('DS5 packaged Durable User State release', () => {
  test('a clean consumer installs WOML and remembers state across native process restarts', async () => {
    const packageDirectory = join(temporaryDirectory, 'package');
    const consumerDirectory = join(temporaryDirectory, 'consumer');
    const bunTemporaryDirectory = join(temporaryDirectory, 'bun-temp');
    const bunCacheDirectory = join(temporaryDirectory, 'bun-cache');
    await Promise.all(
      [packageDirectory, consumerDirectory, bunTemporaryDirectory, bunCacheDirectory].map(
        directory => mkdir(directory, { recursive: true })
      )
    );
    await Promise.all([
      Bun.write(
        join(consumerDirectory, 'package.json'),
        JSON.stringify({ name: 'woml-ds5-clean-consumer', private: true })
      ),
      Bun.write(join(consumerDirectory, 'counter.woml'), counterWorkflow),
      Bun.write(join(consumerDirectory, 'conversation.woml'), conversationWorkflow),
    ]);

    const packed = Bun.spawnSync(
      [
        Bun.which('bun')!,
        'pm',
        'pack',
        '--ignore-scripts',
        '--destination',
        packageDirectory,
      ],
      { cwd: packageRoot, stdout: 'pipe', stderr: 'pipe' }
    );
    expect(packed.exitCode, packed.stderr.toString()).toBe(0);
    const archive = (await readdir(packageDirectory))
      .filter(name => name.endsWith('.tgz'))
      .map(name => join(packageDirectory, name))[0];
    expect(archive).toBeDefined();

    const installed = Bun.spawnSync([Bun.which('bun')!, 'add', archive!, '--no-save'], {
      cwd: consumerDirectory,
      env: {
        ...process.env,
        TMPDIR: bunTemporaryDirectory,
        BUN_INSTALL_CACHE_DIR: bunCacheDirectory,
      },
      stdout: 'pipe',
      stderr: 'pipe',
    });
    expect(
      installed.exitCode,
      `${installed.stdout.toString()}${installed.stderr.toString()}`
    ).toBe(0);

    const executable = join(consumerDirectory, 'node_modules', '.bin', 'woml');
    const statePath = join(consumerDirectory, 'state.sqlite');
    const invoke = (...args: string[]) =>
      Bun.spawnSync([executable, ...args], {
        cwd: consumerDirectory,
        stdout: 'pipe',
        stderr: 'pipe',
      });

    const checked = invoke('check', 'counter.woml');
    expect(checked.exitCode, checked.stderr.toString()).toBe(0);
    expect(checked.stdout.toString()).toContain('WOML check passed');
    const declarations = await Bun.file(
      join(consumerDirectory, 'woml-env.d.ts')
    ).text();
    expect(declarations).toContain('interface WomlStateService');

    const firstCounter = invoke('test', 'counter.woml', '--state', statePath);
    const secondCounter = invoke('test', 'counter.woml', '--state', statePath);
    expect(firstCounter.exitCode, firstCounter.stderr.toString()).toBe(0);
    expect(secondCounter.exitCode, secondCounter.stderr.toString()).toBe(0);
    expect(JSON.parse(firstCounter.stdout.toString())).toEqual({ count: 1, version: 1 });
    expect(JSON.parse(secondCounter.stdout.toString())).toEqual({ count: 2, version: 2 });

    const firstConversation = invoke('test', 'conversation.woml', '--state', statePath);
    const secondConversation = invoke('test', 'conversation.woml', '--state', statePath);
    expect(firstConversation.exitCode, firstConversation.stderr.toString()).toBe(0);
    expect(secondConversation.exitCode, secondConversation.stderr.toString()).toBe(0);
    expect(JSON.parse(firstConversation.stdout.toString())).toEqual({ turns: 1, version: 1 });
    expect(JSON.parse(secondConversation.stdout.toString())).toEqual({ turns: 2, version: 2 });

    const listed = invoke('list', '--state', statePath, '--json');
    expect(listed.exitCode, listed.stderr.toString()).toBe(0);
    const inspectionSurface = listed.stdout.toString();
    expect(inspectionSurface).not.toContain('private-counter-key');
    expect(inspectionSurface).not.toContain('private-conversation-key');

    if (process.platform !== 'win32') {
      expect((await stat(statePath)).mode & 0o777).toBe(0o600);
    }
  }, 60_000);
});
