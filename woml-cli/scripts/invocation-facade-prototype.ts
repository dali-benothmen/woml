#!/usr/bin/env bun
// Diagnostic-only ISR2. Build and capture are separate explicit operations.
import assert from 'node:assert/strict';
import { copyFile, mkdir, mkdtemp, readFile } from 'node:fs/promises';
import { arch, cpus, hostname, platform, release } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import contract from '../../docs/invocation-facade-prototype.v1.json';
import acceptance from '../../docs/invocation-setup-acceptance.v1.json';
import { originalWorkerPath, prototypeFacade } from './invocation-facade-transform';
import { sha256 } from './invocation-setup-fixtures';
import { summarizeInvocationBaseline, validateInvocationCapture } from './invocation-setup-measurement';
import { headroom, resourceRecord, snapshot } from './validated-state-baseline';

const root = resolve(import.meta.dir, '../..');
const cli = join(root, 'woml-cli');
const files = ['script-host.js', 'script-host-worker.js', 'script-host.js.map', 'script-host-worker.js.map'];
const paths = ['docs/invocation-facade-prototype.v1.json', 'docs/invocation-setup-acceptance.v1.json',
  'woml-cli/scripts/invocation-facade-prototype.ts', 'woml-cli/scripts/invocation-facade-transform.ts',
  'woml-cli/scripts/invocation-setup-child.ts', 'woml-cli/scripts/invocation-setup-fixtures.ts',
  'woml-cli/scripts/invocation-setup-measurement.ts', 'woml-cli/scripts/validated-state-baseline.ts',
  'woml-cli/scripts/validated-state-comparison.ts', 'woml-cli/scripts/build-javascript.ts'];
const machine = () => ({ platform: platform(), arch: arch(), hostname: hostname(), cpu: cpus()[0]?.model, kernel: release() });
async function inputs() {
  return { source: await snapshot(), harness: await Promise.all(paths.map(async path => ({ path, sha256: sha256(await readFile(join(root, path))) }))) };
}
async function directory(prefix: string) {
  const parent = join(cli, '.woml/performance'); await mkdir(parent, { recursive: true });
  return mkdtemp(join(parent, prefix));
}
export function assertRebuiltArtifact(name: string, generated: string, original: string, generatedRoot: string, originalRoot: string) {
  // Bun embeds output-relative source paths and their debugId in the artifact.
  // A relocated build changes only these metadata fields, not executable code.
  if (!name.endsWith('.map')) {
    const split = (source: string) => {
      assert.match(source, /\n\/\/# debugId=[A-F0-9]+\n$/);
      return source.replace(/\n\/\/# debugId=[A-F0-9]+\n$/, '');
    };
    assert.equal(split(generated), split(original), `Executable reference drift: ${name}`);
  } else {
    const normalize = (text: string, base: string) => {
      const map = JSON.parse(text); assert.match(map.debugId, /^[A-F0-9]+$/);
      const { debugId, ...rest } = map;
      return { ...rest, sources: map.sources.map((path: string) => resolve(base, path)) };
    };
    assert.deepEqual(normalize(generated, generatedRoot), normalize(original, originalRoot), `Source-map reference drift: ${name}`);
  }
}
function environment() {
  for (const name of ['WOML_PROFILE', 'WOML_PROFILE_OUTPUT', 'LD_PRELOAD', 'DYLD_INSERT_LIBRARIES', 'RUST_MIN_STACK', 'NODE_OPTIONS', 'BUN_OPTIONS']) assert.ok(!process.env[name], `Unset ${name}`);
}

export function summarizeFacade(reference: any[], candidate: any[]) {
  const before = summarizeInvocationBaseline(reference), after = summarizeInvocationBaseline(candidate);
  const rows = before.rows.map((row, index) => {
    const next = after.rows[index]!;
    assert.equal(row.fixtureSha256, next.fixtureSha256); assert.ok(row.batchMedianMs > 0);
    const improvementPercent = (row.batchMedianMs - next.batchMedianMs) / row.batchMedianMs * 100;
    return { cohort: row.cohort, reference: row, candidate: next, improvementPercent,
      controlInvestigationRequired: row.cohort !== acceptance.primary.cohort && -improvementPercent > acceptance.investigation.controlMedianRegressionPercent };
  });
  const primary = rows.find(row => row.cohort === acceptance.primary.cohort)!;
  return { profile: 'woml.invocation-facade-comparison/v1', rows,
    numericalPrimaryTargetMet: primary.improvementPercent >= acceptance.primary.minimumImprovementPercent,
    controlInvestigations: rows.filter(row => row.controlInvestigationRequired).map(row => row.cohort),
    releaseAccepted: false, integrationAccepted: false, allocationBytes: null, scopedPeakLiveBytes: null, cpuMs: null,
    limitation: 'Untraced direct Bun-host comparison only. Numerical target does not certify isolation, resources, native/installed execution or platform release prerequisites.' };
}

async function build(baselinePath: string) {
  environment(); await headroom(); const before = await inputs();
  const ancestry = Bun.spawnSync(['git', 'merge-base', '--is-ancestor', contract.sourceCheckpoint, before.source.commit], { cwd: root });
  assert.equal(ancestry.exitCode, 0);
  const baseline = JSON.parse(await readFile(resolve(baselinePath), 'utf8'));
  assert.equal(baseline.profile, 'woml.invocation-setup-build/v1'); assert.equal(baseline.mode, 'baseline');
  assert.deepEqual(before.source.files, baseline.inputs.source.files, 'Production source must match independent original reference');
  assert.deepEqual(machine(), baseline.host); assert.equal(Bun.version, baseline.toolchain.bun);
  assert.equal(sha256(await readFile(process.execPath)), baseline.bunExecutableSha256);
  for (const name of files) {
    const original = baseline.artifacts.find((file: any) => file.name === name); assert.ok(original);
    assert.equal(sha256(await readFile(original.path)), original.sha256);
  }
  const source = await readFile(originalWorkerPath, 'utf8'), transformed = prototypeFacade(source);
  const output = await directory('invocation-facade-build-');
  // Standalone generated typecheck tree: no source override or new dependency.
  const checkRoot = join(output, 'typecheck'); await mkdir(join(checkRoot, 'script-host'), { recursive: true });
  await Bun.write(join(checkRoot, 'script-host/worker.ts'), transformed.source);
  for (const path of ['script-host/json.ts', 'script-host/types.ts', 'performance-profiler.ts']) await copyFile(join(cli, 'src', path), join(checkRoot, path));
  const check = Bun.spawn([process.execPath, join(cli, 'node_modules/typescript/bin/tsc'), '--ignoreConfig', '--noEmit', '--target', 'ES2022',
    '--module', 'ESNext', '--moduleResolution', 'Bundler', '--lib', 'ES2022,WebWorker', '--strict', '--skipLibCheck', '--types', 'bun',
    join(checkRoot, 'script-host/worker.ts')], { cwd: cli, stdout: 'pipe', stderr: 'pipe' });
  const stdout = new Response(check.stdout).text(), stderr = new Response(check.stderr).text();
  const code = await check.exited; await Bun.write(join(output, 'typecheck.log'), `${await stdout}\n${await stderr}`);
  assert.equal(code, 0, `Candidate typecheck failed; retained ${output}`);
  const artifacts = [];
  for (const variant of ['reference', 'candidate'] as const) {
    const outdir = join(output, `generated-${variant}`), retained = join(output, variant); await mkdir(outdir); await mkdir(retained);
    const result = await Bun.build({
      entrypoints: ['cli', 'script-host', 'script-host-worker', 'notification-provider-host', 'custom-notification-provider-host',
        'custom-notification-provider-worker', 'opentelemetry-runtime', 'opentelemetry-sdk'].map(name => join(cli, 'src', `${name}.ts`)),
      outdir, target: 'bun', format: 'esm', sourcemap: 'external', minify: true, naming: '[name].[ext]',
      external: ['./opentelemetry-runtime.ts', './opentelemetry-runtime.js', './opentelemetry-sdk.ts', './opentelemetry-sdk.js'],
      plugins: variant === 'reference' ? [] : [{ name: 'private-facade-prototype', setup(build) {
        build.onLoad({ filter: /[/\\]script-host[/\\]worker\.ts$/ }, args => {
          assert.equal(resolve(args.path), originalWorkerPath);
          return { contents: transformed.source, loader: 'ts', resolveDir: dirname(originalWorkerPath) };
        });
      } }],
    });
    assert.ok(result.success, result.logs.map(log => log.message).join('\n'));
    for (const name of files) {
      const generatedPath = join(outdir, name), path = join(retained, name);
      const original = baseline.artifacts.find((file: any) => file.name === name);
      if (variant === 'reference' || name.startsWith('script-host.js')) {
        assertRebuiltArtifact(name, await readFile(generatedPath, 'utf8'), await readFile(original.path, 'utf8'), outdir, join(cli, 'dist'));
        // Measure the exact independent archive, not a rewritten reference.
        // The host is byte-identical in both measured variants.
        await copyFile(original.path, path);
      } else await copyFile(generatedPath, path);
      const bytes = await readFile(path), digest = sha256(bytes);
      if (variant === 'reference') assert.equal(digest, original.sha256);
      if (name === 'script-host.js') assert.equal(digest, contract.originalHostSha256);
      artifacts.push({ variant, name, path, sha256: digest, bytes: bytes.length });
    }
  }
  assert.notEqual(artifacts.find(file => file.variant === 'candidate' && file.name === 'script-host-worker.js')!.sha256, contract.originalWorkerSha256);
  await Bun.write(join(output, 'original-worker.ts'), source);
  await Bun.write(join(output, 'prototype-worker.ts'), transformed.source);
  for (const file of before.harness) {
    const target = join(output, 'harness', file.path); await mkdir(dirname(target), { recursive: true }); await copyFile(join(root, file.path), target);
  }
  assert.deepEqual(await inputs(), before);
  const receipt = { profile: 'woml.invocation-facade-build/v1', inputs: before, baseline, artifacts, host: machine(), bun: Bun.version,
    bunExecutableSha256: baseline.bunExecutableSha256, transformedSourceSha256: sha256(transformed.source), originalBodySha256: sha256(transformed.originalBody),
    isolation: transformed.rationale, releaseAccepted: false };
  await Bun.write(join(output, 'receipt.json'), JSON.stringify(receipt, null, 2)); console.log(`Prototype build receipt: ${join(output, 'receipt.json')}`);
}

async function capture(receiptPath: string) {
  environment(); const receipt = JSON.parse(await readFile(resolve(receiptPath), 'utf8'));
  assert.equal(receipt.profile, 'woml.invocation-facade-build/v1'); assert.deepEqual(receipt.host, machine()); assert.equal(receipt.bun, Bun.version);
  const check = async () => {
    assert.deepEqual(await inputs(), receipt.inputs);
    assert.equal(sha256(await readFile(process.execPath)), receipt.bunExecutableSha256);
    assert.equal(sha256(prototypeFacade(await readFile(originalWorkerPath, 'utf8')).source), receipt.transformedSourceSha256);
    for (const file of receipt.artifacts) assert.equal(sha256(await readFile(file.path)), file.sha256);
  };
  await check(); const output = await directory('invocation-facade-capture-');
  await Bun.write(join(output, 'inputs.json'), JSON.stringify({ receipt, acceptance, contract }, null, 2));
  const reports: Record<'reference' | 'candidate', any[]> = { reference: [], candidate: [] };
  for (let pair = 0; pair < acceptance.processesPerArtifact; pair++) {
    const order = pair % 2 === 0 ? ['reference', 'candidate'] as const : ['candidate', 'reference'] as const;
    for (const variant of order) {
      await check(); const before = await headroom(), working = join(output, `pair${pair}-${variant}`); await mkdir(working);
      const host = receipt.artifacts.find((file: any) => file.variant === variant && file.name === 'script-host.js').path;
      const child = Bun.spawn([process.execPath, join(cli, 'scripts/invocation-setup-child.ts'), host, working, 'timing'],
        { cwd: root, stdout: 'pipe', stderr: 'pipe', timeout: 900_000 });
      const stdout = new Response(child.stdout).text(), stderr = new Response(child.stderr).text(), exitCode = await child.exited;
      await Bun.write(join(working, 'process.log'), `${await stdout}\n${await stderr}`);
      await Bun.write(join(working, 'resources.json'), JSON.stringify({ before, exitCode, usage: resourceRecord(child.resourceUsage()) }, null, 2));
      assert.equal(exitCode, 0, `Failed process retained at ${working}`);
      const report = JSON.parse(await readFile(join(working, 'capture.json'), 'utf8')); validateInvocationCapture(report); reports[variant].push(report);
      await check(); console.log(`Matched pair ${pair + 1}/${acceptance.processesPerArtifact}: ${variant} complete`);
    }
  }
  await Bun.write(join(output, 'summary.json'), JSON.stringify(summarizeFacade(reports.reference, reports.candidate), null, 2));
  console.log(`Prototype comparison: ${output}`);
}
if (import.meta.main) {
  const [mode, flag, path] = process.argv.slice(2);
  assert.ok(process.argv.length === 5 && path && ((mode === 'build' && flag === '--baseline') || (mode === 'capture' && flag === '--receipt')),
    'Use build --baseline ORIGINAL_ISR_RECEIPT or capture --receipt PROTOTYPE_RECEIPT');
  if (mode === 'build') await build(path); else await capture(path);
}
