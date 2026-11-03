#!/usr/bin/env bun
// Reopened ISR1 diagnostic. Original ISR0 runner/acceptance stay frozen.
import assert from 'node:assert/strict';
import { copyFile, mkdir, mkdtemp, readFile } from 'node:fs/promises';
import { arch, cpus, hostname, platform, release } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import contract from '../../docs/invocation-bootstrap-diagnostic.v1.json';
import { bootstrapFixture, bootstrapOrder } from './invocation-bootstrap-child';
import { sha256 } from './invocation-setup-fixtures';
import { headroom, resourceRecord, snapshot } from './validated-state-baseline';
import { median } from './validated-state-comparison';

const root = resolve(import.meta.dir, '../..');
const fixtureHashes = new Map(contract.cases.map(name => [name, bootstrapFixture(name).fixtureSha256]));
const paths = ['docs/invocation-bootstrap-diagnostic.v1.json', 'woml-cli/scripts/invocation-bootstrap.ts',
  'woml-cli/scripts/invocation-bootstrap-child.ts', 'woml-cli/scripts/invocation-bootstrap-worker.ts',
  'woml-cli/scripts/invocation-setup-fixtures.ts', 'docs/invocation-setup-acceptance.v1.json',
  'woml-cli/scripts/validated-state-baseline.ts', 'woml-cli/scripts/validated-state-comparison.ts'];
const host = () => ({ platform: platform(), arch: arch(), hostname: hostname(), cpu: cpus()[0]?.model, kernel: release() });
async function inputs() {
  return { source: await snapshot(), harness: await Promise.all(paths.map(async path => ({ path, sha256: sha256(await readFile(join(root, path))) }))) };
}
async function directory(prefix: string) {
  const parent = join(root, 'woml-cli/.woml/performance'); await mkdir(parent, { recursive: true });
  return await mkdtemp(join(parent, prefix));
}
function environment() {
  for (const name of ['WOML_PROFILE', 'WOML_PROFILE_OUTPUT', 'LD_PRELOAD', 'DYLD_INSERT_LIBRARIES', 'RUST_MIN_STACK', 'NODE_OPTIONS', 'BUN_OPTIONS']) assert.ok(!process.env[name], `Unset ${name}`);
}

export function validateBootstrap(value: any) {
  assert.equal(value.profile, 'woml.invocation-bootstrap-capture/v1'); assert.equal(value.releaseAccepted, false);
  assert.ok(Number.isInteger(value.processIndex) && value.processIndex >= 0 && value.processIndex < contract.processes);
  assert.equal(value.cpuMs, null); assert.equal(value.allocatedBytes, null); assert.equal(value.peakLiveBytes, null);
  assert.equal(value.rows.length, contract.cases.length * (contract.warmupSamples + contract.measuredSamples));
  const observers = new Set(), realms = new Set();
  for (const [position, row] of value.rows.entries()) {
    const sample = Math.floor(position / contract.cases.length);
    assert.equal(row.name, bootstrapOrder(sample, value.processIndex)[position % contract.cases.length]);
    assert.equal(row.sample, sample); assert.equal(row.warmup, sample < contract.warmupSamples);
    assert.ok(Number.isFinite(row.batchMs) && row.batchMs >= 0);
    assert.equal(row.workers.length, row.name === 'wrapped-concurrent' ? contract.concurrentWorkers : 1);
    for (const worker of row.workers) {
      assert.equal(worker.name, row.name); assert.equal(worker.sample, sample); assert.equal(worker.warmup, row.warmup);
      assert.equal(worker.fixtureSha256, fixtureHashes.get(row.name));
      assert.ok(!observers.has(worker.observerId)); observers.add(worker.observerId);
      assert.equal(worker.executedProductionCode, row.name !== 'wrapped-empty');
      for (const name of ['constructorMs', 'requestRoundTripMs', 'journeyMs']) assert.ok(Number.isFinite(worker[name]) && worker[name] >= 0);
      const raw = row.name.startsWith('raw-');
      if (raw) {
        for (const name of ['realmId', 'enteredMs', 'readyMs', 'importMs', 'executionMs']) assert.equal(worker[name], null);
      } else {
        assert.match(worker.realmId, /^[a-f0-9-]{36}$/); assert.ok(!realms.has(worker.realmId)); realms.add(worker.realmId);
        for (const name of ['enteredMs', 'readyMs', 'executionMs']) assert.ok(Number.isFinite(worker[name]) && worker[name] >= 0);
        assert.ok(worker.enteredMs <= worker.readyMs && worker.readyMs <= worker.journeyMs);
        if (row.name === 'wrapped-empty') assert.equal(worker.importMs, null);
        else assert.ok(Number.isFinite(worker.importMs) && worker.importMs >= 0);
      }
    }
  }
}

export function summarizeBootstrap(reports: any[]) {
  assert.equal(reports.length, contract.processes); reports.forEach(validateBootstrap);
  assert.deepEqual(reports.map(report => report.processIndex), Array.from({ length: contract.processes }, (_, index) => index));
  return { profile: 'woml.invocation-bootstrap-summary/v1', releaseAccepted: false,
    rows: contract.cases.map(name => ({ name,
      metrics: ['constructorMs', 'enteredMs', 'readyMs', 'importMs', 'executionMs', 'requestRoundTripMs', 'journeyMs'].map(metric => {
        const processes = reports.map(report => {
          const values = report.rows.filter((row: any) => row.name === name && !row.warmup).flatMap((row: any) => row.workers.map((worker: any) => worker[metric]));
          return values.includes(null) ? null : median(values);
        });
        return { metric, processMedians: processes, medianMs: processes.includes(null) ? null : median(processes as number[]) };
      }) })),
    limitation: 'Raw and wrapper paths are different diagnostics. Wrapper adds a configure/ready handshake and dynamic import. Do not subtract these medians to claim exact raw startup sub-costs or a speedup.' };
}

async function build(baselinePath: string) {
  environment(); await headroom(); const before = await inputs();
  // A later metadata-only commit may retain the exact production bytes. Require
  // reviewed ancestry AND file-by-file identity; never rebase the old receipt.
  const ancestry = Bun.spawnSync(['git', 'merge-base', '--is-ancestor', contract.sourceCheckpoint, before.source.commit], { cwd: root });
  assert.equal(ancestry.exitCode, 0, 'Diagnostic lineage changed; review provenance explicitly');
  const baseline = JSON.parse(await readFile(resolve(baselinePath), 'utf8'));
  assert.equal(baseline.profile, 'woml.invocation-setup-build/v1'); assert.equal(baseline.mode, 'baseline');
  assert.equal(baseline.inputs.source.sha256, contract.originalSourceSha256);
  assert.equal(sha256(JSON.stringify(baseline.inputs.source.files)), contract.originalSourceSha256);
  assert.equal(before.source.sha256, contract.originalSourceSha256);
  assert.deepEqual(before.source.files, baseline.inputs.source.files, 'Production content differs from original worker baseline');
  assert.deepEqual(baseline.host, host()); assert.equal(baseline.toolchain.bun, Bun.version);
  assert.equal(sha256(await readFile(process.execPath)), baseline.bunExecutableSha256);
  const original = baseline.artifacts.find((file: any) => file.name === 'script-host-worker.js');
  assert.equal(original.sha256, contract.originalWorkerSha256);
  assert.equal(sha256(await readFile(original.path)), contract.originalWorkerSha256);
  const output = await directory('invocation-bootstrap-build-');
  const wrapper = join(output, 'invocation-bootstrap-worker.js');
  const result = await Bun.build({ entrypoints: [join(root, 'woml-cli/scripts/invocation-bootstrap-worker.ts')],
    outdir: output, target: 'bun', format: 'esm', minify: false, sourcemap: 'external', naming: '[name].[ext]' });
  assert.ok(result.success, result.logs.map(log => log.message).join('\n'));
  const worker = join(output, 'original-worker.js'); await copyFile(original.path, worker);
  assert.deepEqual(await inputs(), before);
  for (const file of before.harness) {
    const path = join(output, 'harness', file.path); await mkdir(dirname(path), { recursive: true }); await copyFile(join(root, file.path), path);
  }
  const receipt = { profile: 'woml.invocation-bootstrap-build/v1', baseline, inputs: before, host: host(), bun: Bun.version,
    bunExecutableSha256: baseline.bunExecutableSha256, worker, workerSha256: contract.originalWorkerSha256,
    wrapper, wrapperSha256: sha256(await readFile(wrapper)), releaseAccepted: false };
  await Bun.write(join(output, 'receipt.json'), JSON.stringify(receipt, null, 2));
  console.log(`Bootstrap build receipt: ${join(output, 'receipt.json')}`);
}
async function capture(path: string) {
  environment(); const receipt = JSON.parse(await readFile(resolve(path), 'utf8'));
  assert.equal(receipt.profile, 'woml.invocation-bootstrap-build/v1'); assert.deepEqual(receipt.host, host()); assert.equal(receipt.bun, Bun.version);
  const check = async () => {
    assert.deepEqual(await inputs(), receipt.inputs);
    assert.equal(sha256(await readFile(process.execPath)), receipt.bunExecutableSha256);
    assert.equal(sha256(await readFile(receipt.worker)), contract.originalWorkerSha256); assert.equal(receipt.workerSha256, contract.originalWorkerSha256);
    assert.equal(sha256(await readFile(receipt.wrapper)), receipt.wrapperSha256);
  };
  await check(); const output = await directory('invocation-bootstrap-capture-');
  await Bun.write(join(output, 'inputs.json'), JSON.stringify({ receipt, contract }, null, 2));
  const reports = [];
  for (let index = 0; index < contract.processes; index++) {
    await check(); const before = await headroom(); const working = join(output, `process${index}`); await mkdir(working);
    const child = Bun.spawn([process.execPath, join(root, 'woml-cli/scripts/invocation-bootstrap-child.ts'), receipt.worker, receipt.wrapper, working, String(index)],
      { cwd: root, stdout: 'pipe', stderr: 'pipe', timeout: 300_000 });
    const stdout = new Response(child.stdout).text(), stderr = new Response(child.stderr).text(), exitCode = await child.exited;
    await Bun.write(join(working, 'process.log'), `${await stdout}\n${await stderr}`);
    await Bun.write(join(working, 'resources.json'), JSON.stringify({ before, exitCode, usage: resourceRecord(child.resourceUsage()) }, null, 2));
    assert.equal(exitCode, 0, `Failed diagnostic retained at ${working}`);
    const report = JSON.parse(await readFile(join(working, 'capture.json'), 'utf8')); validateBootstrap(report); reports.push(report);
    await check(); console.log(`Bootstrap diagnostic ${index + 1}/${contract.processes}`);
  }
  await Bun.write(join(output, 'summary.json'), JSON.stringify(summarizeBootstrap(reports), null, 2));
  console.log(`Bootstrap evidence: ${output}`);
}
if (import.meta.main) {
  const [mode, flag, path] = process.argv.slice(2);
  assert.ok(process.argv.length === 5 && path && ((mode === 'build' && flag === '--baseline') || (mode === 'capture' && flag === '--receipt')),
    'Use build --baseline ORIGINAL_ISR_RECEIPT or capture --receipt DIAGNOSTIC_RECEIPT');
  if (mode === 'build') await build(path); else await capture(path);
}
