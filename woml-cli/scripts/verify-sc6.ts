#!/usr/bin/env bun

import { resolve } from 'node:path';

import { compileWoml, parseWoml } from 'woml';

const cliRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(cliRoot, '..');
const examplePath = resolve(
  projectRoot,
  'examples/httpComparisonWorkflow.woml'
);
const example = await Bun.file(examplePath).text();
const compiled = compileWoml(parseWoml(example, { file: examplePath }));
if (
  compiled.schemaVersion !== 8 ||
  !compiled.graph.nodes.some(
    node => node.scriptRuntime?.bindings.includes('services') === true
  )
) {
  throw new Error(
    'SC6 verification failed: the HTTP comparison example must compile through Model v8 and Script Bindings v1.'
  );
}

const documentation = await Bun.file(
  resolve(projectRoot, 'docs/woml-http-services.md')
).text();
for (const section of [
  '## Choosing the HTTP path',
  '## Failure and retry behavior',
  '## Durable and secret boundaries',
  '## SSRF and network policy',
  '## Deployment checklist',
  '## Benchmarking',
]) {
  if (!documentation.includes(section)) {
    throw new Error(`SC6 HTTP documentation is missing ${section}.`);
  }
}
const architecture = await Bun.file(
  resolve(projectRoot, 'docs/architecture.md')
).text();
const language = await Bun.file(
  resolve(projectRoot, 'docs/woml-v0.1.md')
).text();
if (
  !architecture.includes('workflow-scoped Cache v1 are active through SC10') ||
  !language.includes('SC0–SC10 implemented and hardened')
) {
  throw new Error('Services architecture or language status is stale.');
}

process.stdout.write(
  '[SC6] HTTP example and language/security/deployment documentation are publishable\n'
);

// T13 reaches the existing complete transitive release gate. N6 rebuilds the
// package and runs every frontend, Rust, isolated CLI, clean-package, recovery,
// crash, contention, lint, type, and secret-safety suite. SC6 extends those
// suites with HTTP composition and the clean-package HTTP example journey.
await import('./verify-t13.ts');

const benchmark = Bun.spawn(
  [
    Bun.which('bun')!,
    resolve(import.meta.dir, 'benchmark-sc6-http.ts'),
    '--iterations',
    '2',
    '--warmup',
    '0',
  ],
  { cwd: cliRoot, stdout: 'pipe', stderr: 'pipe' }
);
const [stdout, stderr, exitCode] = await Promise.all([
  new Response(benchmark.stdout).text(),
  new Response(benchmark.stderr).text(),
  benchmark.exited,
]);
if (exitCode !== 0) {
  throw new Error(`SC6 benchmark smoke failed:\n${stderr}`);
}
const report = JSON.parse(stdout) as Record<string, unknown>;
if (
  report.benchmark !== 'woml-http-local-loopback-v1' ||
  typeof report.managedToNativeSequentialRatio !== 'number' ||
  !Number.isFinite(report.managedToNativeSequentialRatio)
) {
  throw new Error('SC6 benchmark did not produce the frozen report shape.');
}

process.stdout.write('[SC6] HTTP capability foundation release gate passed\n');
