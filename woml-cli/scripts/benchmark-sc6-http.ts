#!/usr/bin/env bun

import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

interface ProfileResult {
  readonly sequentialMs: number;
  readonly concurrentMs: number;
  readonly iterations: number;
}

const packageRoot = resolve(import.meta.dir, '..');
const cli = resolve(packageRoot, 'dist', 'cli.js');

function integerOption(name: string, fallback: number): number {
  const index = process.argv.indexOf(name);
  if (index === -1) return fallback;
  const value = Number(process.argv[index + 1]);
  if (!Number.isSafeInteger(value)) {
    throw new Error(`${name} requires an integer.`);
  }
  return value;
}

const iterations = integerOption('--iterations', 20);
const warmup = integerOption('--warmup', 3);
if (iterations < 1 || iterations > 32 || warmup < 0 || warmup > 10) {
  throw new Error(
    'SC6 benchmark bounds are --iterations 1..32 and --warmup 0..10.'
  );
}
if (!(await Bun.file(cli).exists())) {
  throw new Error('Build the packaged CLI first with: bun run build');
}

const directory = await mkdtemp(join(tmpdir(), 'woml-sc6-http-benchmark-'));
let requests = 0;
const server = Bun.serve({
  hostname: '127.0.0.1',
  port: 0,
  fetch: () => {
    requests += 1;
    return Response.json({ ok: true });
  },
});
const url = new URL('/benchmark', server.url).toString();

function workflow(mode: 'native' | 'managed'): string {
  const request =
    mode === 'native'
      ? `const response = await fetch(${JSON.stringify(url)}); await response.json();`
      : `await services.http.request({ url: ${JSON.stringify(url)} });`;
  const concurrent =
    mode === 'native'
      ? `fetch(${JSON.stringify(url)}).then(response => response.json())`
      : `services.http.request({ url: ${JSON.stringify(url)} })`;
  return `<workflow version="0.1" id="sc6-${mode}-benchmark" name="SC6 ${mode} benchmark">
  <triggers><manual id="start" /></triggers>
  <steps>
    <step id="benchmark">
      <script>
        for (let index = 0; index < ${warmup}; index += 1) { ${request} }
        const sequentialStart = performance.now();
        for (let index = 0; index < ${iterations}; index += 1) { ${request} }
        const sequentialMs = performance.now() - sequentialStart;
        const concurrentStart = performance.now();
        await Promise.all(Array.from({ length: ${iterations} }, () => ${concurrent}));
        const concurrentMs = performance.now() - concurrentStart;
        return { sequentialMs, concurrentMs, iterations: ${iterations} };
      </script>
    </step>
  </steps>
</workflow>`;
}

async function run(mode: 'native' | 'managed'): Promise<ProfileResult> {
  const path = join(directory, `${mode}.woml`);
  await writeFile(path, workflow(mode));
  const child = Bun.spawn(
    [cli, 'test', path, '--state', join(directory, `${mode}.sqlite`)],
    { cwd: directory, stdout: 'pipe', stderr: 'pipe' }
  );
  const [stdout, stderr, exitCode] = await Promise.all([
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
    child.exited,
  ]);
  if (exitCode !== 0) {
    throw new Error(`The ${mode} benchmark failed:\n${stderr}`);
  }
  return JSON.parse(stdout) as ProfileResult;
}

try {
  const native = await run('native');
  const managed = await run('managed');
  const expectedRequests = 2 * (warmup + iterations + iterations);
  if (requests !== expectedRequests) {
    throw new Error(
      `Expected ${expectedRequests} local requests but received ${requests}.`
    );
  }
  const report = {
    benchmark: 'woml-http-local-loopback-v1',
    note: 'Measured inside one script attempt; CLI and process startup are excluded.',
    iterations,
    warmup,
    native: {
      sequentialTotalMs: native.sequentialMs,
      sequentialMeanMs: native.sequentialMs / iterations,
      concurrentTotalMs: native.concurrentMs,
      concurrentRequestsPerSecond: (iterations * 1_000) / native.concurrentMs,
    },
    managed: {
      sequentialTotalMs: managed.sequentialMs,
      sequentialMeanMs: managed.sequentialMs / iterations,
      concurrentTotalMs: managed.concurrentMs,
      concurrentRequestsPerSecond: (iterations * 1_000) / managed.concurrentMs,
    },
    managedToNativeSequentialRatio: managed.sequentialMs / native.sequentialMs,
  };
  process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
} finally {
  server.stop(true);
  await rm(directory, { recursive: true, force: true });
}
