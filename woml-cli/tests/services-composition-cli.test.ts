import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { chmod, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const packageRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(packageRoot, '..');
const cliPath = join(packageRoot, 'dist', 'cli.js');
const exampleDirectory = join(projectRoot, 'examples', 'servicesComposition');
let temporaryDirectory: string;

beforeAll(async () => {
  const build = Bun.spawnSync([Bun.which('bun')!, 'run', 'build'], {
    cwd: packageRoot,
    stdout: 'pipe',
    stderr: 'pipe',
  });
  if (build.exitCode !== 0) {
    throw new Error(
      `Could not build the  CLI:\n${build.stdout.toString()}${build.stderr.toString()}`
    );
  }
  await chmod(cliPath, 0o755);
  temporaryDirectory = await mkdtemp(join(tmpdir(), 'woml-sc14-cli-'));
  await mkdir(join(temporaryDirectory, '.woml'));
}, 120_000);

afterAll(async () => {
  if (temporaryDirectory !== undefined) {
    await rm(temporaryDirectory, { recursive: true, force: true });
  }
});

describe('Packaged services composition', () => {
  test('runs database, storage, cache, and internal events through one active runtime', async () => {
    const statePath = join(temporaryDirectory, '.woml', 'state.sqlite');
    const probe = Bun.serve({ hostname: '127.0.0.1', port: 0, fetch: () => new Response() });
    const port = probe.port!;
    probe.stop(true);
    const publisherPath = join(temporaryDirectory, 'publisher-webhook.woml');
    await writeFile(
      publisherPath,
      (await readFile(join(exampleDirectory, 'publisher.woml'), 'utf8')).replace(
        '<manual id="start" />',
        '<webhook id="start" path="/publish-order" method="POST" auth="none" />'
      )
    );
    const child = Bun.spawn(
      [
        cliPath,
        'run',
        publisherPath,
        join(exampleDirectory, 'subscriber.woml'),
        '--state',
        statePath,
        '--port',
        String(port),
      ],
      { cwd: temporaryDirectory, stdout: 'pipe', stderr: 'pipe' }
    );
    let stdout = '';
    let stderr = '';
    const consume = async (
      stream: ReadableStream<Uint8Array>,
      append: (text: string) => void
    ) => {
      const reader = stream.getReader();
      const decoder = new TextDecoder();
      while (true) {
        const chunk = await reader.read();
        if (chunk.done) break;
        append(decoder.decode(chunk.value, { stream: true }));
      }
      append(decoder.decode());
    };
    const stdoutDone = consume(child.stdout, text => {
      stdout += text;
    });
    const stderrDone = consume(child.stderr, text => {
      stderr += text;
    });
    const waitFor = async (needle: string): Promise<void> => {
      const deadline = Date.now() + 20_000;
      while (!stderr.includes(needle) && !stdout.includes(needle)) {
        if (Date.now() >= deadline) {
          throw new Error(
            `Timed out waiting for ${needle}.\nstdout:\n${stdout}\nstderr:\n${stderr}`
          );
        }
        await Bun.sleep(10);
      }
    };

    try {
      await waitFor('WOML automation is active.');
      const publisher = await fetch(`http://127.0.0.1:${port}/publish-order`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: '{}',
      });
      expect(publisher.status).toBe(202);
      await waitFor('Order order-42 is ready for Alex');
      expect(stderr).toContain('services-order-subscriber');
      expect(stderr).toContain('"eventName": "order.prepared"');
      expect(stdout).toBe('');
      expect(
        await Bun.file(
          join(temporaryDirectory, '.woml', 'services-composition.sqlite')
        ).exists()
      ).toBe(true);
      expect(
        await Bun.file(
          join(temporaryDirectory, '.woml', 'cache-v1.sqlite')
        ).exists()
      ).toBe(true);
      expect(
        (await readdir(join(temporaryDirectory, '.woml', 'objects-v1'))).some(
          name => name.endsWith('.wobj')
        )
      ).toBe(true);
    } finally {
      child.kill('SIGINT');
    }

    expect(await child.exited).toBe(0);
    await Promise.all([stdoutDone, stderrDone]);
    expect(stderr).toContain('WOML automation stopped.');
  }, 45_000);
});
