import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { mkdir, mkdtemp, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const packageRoot = resolve(import.meta.dir, '..');
let temporaryDirectory: string;

function frame(value: unknown): Uint8Array {
  const body = Buffer.from(JSON.stringify(value), 'utf8');
  return Buffer.concat([
    Buffer.from(`Content-Length: ${body.byteLength}\r\n\r\n`, 'ascii'),
    body,
  ]);
}

function decodeFrames(bytes: Uint8Array): unknown[] {
  const frames: unknown[] = [];
  let offset = 0;
  while (offset < bytes.byteLength) {
    const source = Buffer.from(bytes.buffer, bytes.byteOffset + offset);
    const headerEnd = source.indexOf('\r\n\r\n');
    if (headerEnd < 0) throw new Error('Truncated provider-host header.');
    const header = source.subarray(0, headerEnd).toString('ascii');
    const match = /^Content-Length: ([0-9]+)$/m.exec(header);
    if (!match) throw new Error('Missing provider-host Content-Length.');
    const length = Number(match[1]);
    const bodyStart = headerEnd + 4;
    const bodyEnd = bodyStart + length;
    if (bodyEnd > source.byteLength) throw new Error('Truncated provider-host body.');
    frames.push(JSON.parse(source.subarray(bodyStart, bodyEnd).toString('utf8')));
    offset += bodyEnd;
  }
  return frames;
}

beforeAll(async () => {
  temporaryDirectory = await mkdtemp(join(tmpdir(), 'woml-provider-package-'));
});

afterAll(async () => {
  await rm(temporaryDirectory, { recursive: true, force: true });
});

describe('communication-provider clean package', () => {
  test('the installed CLI contains and executes its provider host', async () => {
    const archives = join(temporaryDirectory, 'archives');
    const consumer = join(temporaryDirectory, 'consumer');
    const cache = join(temporaryDirectory, 'bun-cache');
    await Promise.all([archives, consumer, cache].map(path => mkdir(path, { recursive: true })));

    const packed = Bun.spawnSync(
      [Bun.which('bun')!, 'pm', 'pack', '--ignore-scripts', '--destination', archives],
      { cwd: packageRoot, stdout: 'pipe', stderr: 'pipe' }
    );
    expect(packed.exitCode, packed.stderr.toString()).toBe(0);
    const archive = (await readdir(archives))
      .filter(name => name.endsWith('.tgz'))
      .map(name => join(archives, name))[0];
    expect(archive).toBeDefined();

    await writeFile(
      join(consumer, 'package.json'),
      JSON.stringify({ name: 'woml-provider-clean-consumer', private: true })
    );
    const installed = Bun.spawnSync([Bun.which('bun')!, 'add', archive!, '--no-save'], {
      cwd: consumer,
      env: { ...process.env, BUN_INSTALL_CACHE_DIR: cache },
      stdout: 'pipe',
      stderr: 'pipe',
    });
    expect(installed.exitCode, installed.stderr.toString()).toBe(0);

    const installedRoot = join(consumer, 'node_modules/woml');
    for (const artifact of [
      'dist/cli.js',
      'dist/script-host.js',
      'dist/notification-provider-host.js',
    ]) {
      expect(await Bun.file(join(installedRoot, artifact)).exists(), artifact).toBe(true);
    }
    expect(await Bun.file(join(installedRoot, 'src/notification-provider-host.ts')).exists()).toBe(false);

    const executable = join(consumer, 'node_modules/.bin/woml');
    const help = Bun.spawnSync([executable, '--help'], {
      cwd: consumer,
      stdout: 'pipe',
      stderr: 'pipe',
    });
    expect(help.exitCode, help.stderr.toString()).toBe(0);
    const helpText = help.stdout.toString();
    expect(helpText).toContain('telegram doctor');
    expect(helpText).toContain('discord doctor');
    expect(helpText).toContain('whatsapp doctor');

    const runner = join(consumer, 'provider-host-runner.mjs');
    await writeFile(
      runner,
      `import { runNotificationProviderHost } from './node_modules/woml/dist/notification-provider-host.js';\nawait runNotificationProviderHost({ adapter: 'fake' });\n`
    );
    const invocation = await Bun.file(
      resolve(packageRoot, 'tests/fixtures/notification-provider/deliver.v1.json')
    ).json() as { invocationId: string };
    const input = join(consumer, 'provider-host-input.bin');
    await writeFile(input, frame(invocation));
    const host = Bun.spawn([Bun.which('bun')!, runner], {
      cwd: consumer,
      env: {
        ...process.env,
        WOML_SECRETS_PROVIDER: 'env',
        WOML_SECRET_SLACK_BOT_TOKEN: 'xoxb-clean-package-secret',
        WOML_SECRET_SLACK_APP_TOKEN: 'xapp-clean-package-secret',
      },
      stdin: Bun.file(input),
      stdout: 'pipe',
      stderr: 'pipe',
    });
    const [stdout, stderr, exitCode] = await Promise.all([
      new Response(host.stdout).arrayBuffer(),
      new Response(host.stderr).text(),
      host.exited,
    ]);
    expect(exitCode, stderr).toBe(0);
    const messages = decodeFrames(new Uint8Array(stdout));
    expect(messages[0]).toMatchObject({
      protocol: 'woml.notification-provider-host',
      messageType: 'ready',
      providers: ['slack'],
    });
    expect(messages).toContainEqual(
      expect.objectContaining({
        messageType: 'completed',
        invocationId: invocation.invocationId,
        outcome: expect.objectContaining({ kind: 'delivery_success' }),
      })
    );
    expect(JSON.stringify(messages)).not.toContain('clean-package-secret');
    expect(stderr).not.toContain('clean-package-secret');
  }, 60_000);
});
