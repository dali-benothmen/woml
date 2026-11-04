#!/usr/bin/env bun
// ISR0/1 only. No production optimization, implicit capture build, or release pass.
import assert from 'node:assert/strict';
import { copyFile, mkdir, mkdtemp, readFile } from 'node:fs/promises';
import { arch, cpus, hostname, platform, release } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import contract from '../../docs/invocation-setup-acceptance.v1.json';
import { cohorts, fixture, sha256 } from './invocation-setup-fixtures';
import { headroom, resourceRecord, snapshot } from './validated-state-baseline';
import { median } from './validated-state-comparison';

const root = resolve(import.meta.dir, '../..');
const harnessPaths = ['docs/invocation-setup-acceptance.v1.json',
  'woml-cli/scripts/invocation-setup-measurement.ts', 'woml-cli/scripts/invocation-setup-fixtures.ts',
  'woml-cli/scripts/invocation-setup-child.ts', 'woml-cli/scripts/invocation-setup-probe.ts',
  'woml-cli/scripts/invocation-setup-probe-worker.ts',
  'woml-cli/scripts/validated-state-baseline.ts', 'woml-cli/scripts/validated-state-comparison.ts',
  'woml-cli/scripts/build-javascript.ts', 'woml-cli/scripts/release-contract.ts'];
const files = ['script-host.js', 'script-host-worker.js', 'script-host.js.map', 'script-host-worker.js.map'];
const host = () => ({ platform: platform(), arch: arch(), hostname: hostname(), cpu: cpus()[0]?.model, kernel: release() });
const toolchain = () => ({ bun: Bun.version, executable: process.execPath });
async function inputs() {
  return { source: await snapshot(), harness: await Promise.all(harnessPaths.map(async path => ({ path, sha256: sha256(await readFile(join(root, path))) }))) };
}
function git(...args: string[]) {
  const child = Bun.spawnSync(['git', ...args], { cwd: root, stdout: 'pipe', stderr: 'pipe' });
  assert.equal(child.exitCode, 0, child.stderr.toString()); return child.stdout.toString();
}
function baselineSource() {
  assert.equal(git('rev-parse', 'HEAD').trim(), contract.baselineCommit, 'Baseline is frozen to the pre-optimization checkpoint');
  const roots = ['core', 'woml/src', 'woml/tests/fixtures', 'woml-cli/src'];
  assert.equal(git('diff', 'HEAD', '--', ...roots), '', 'Baseline cannot contain production/fixture edits');
  assert.equal(git('ls-files', '--others', '--exclude-standard', '--', ...roots), '', 'Untracked production/fixture sources cannot enter the baseline');
}
function environment() {
  for (const name of ['WOML_PROFILE', 'WOML_PROFILE_OUTPUT', 'LD_PRELOAD', 'DYLD_INSERT_LIBRARIES', 'RUST_MIN_STACK', 'NODE_OPTIONS', 'BUN_OPTIONS']) {
    assert.ok(!process.env[name], `Unset ${name}`);
  }
}
async function directory(prefix: string) {
  const parent = join(root, 'woml-cli/.woml/performance'); await mkdir(parent, { recursive: true });
  return await mkdtemp(join(parent, prefix));
}

export function validateInvocationCapture(value: any, mode = 'timing') {
  assert.equal(value.profile, 'woml.invocation-setup-capture/v1');
  assert.equal(value.mode, mode); assert.equal(value.releaseAccepted, false);
  assert.deepEqual(value.rows.map((row: any) => row.cohort), mode === 'smoke' ? ['mixed'] : cohorts);
  for (const row of value.rows) {
    const spec = fixture(row.cohort);
    assert.equal(row.fixtureSha256, spec.fixtureSha256); assert.equal(row.concurrency, spec.concurrency);
    for (const name of ['readyMs', 'registrationMs', 'firstInvocationMs', 'teardownMs', 'lifetimeMs']) assert.ok(Number.isFinite(row[name]) && row[name] >= 0);
    assert.equal(row.allocationBytes, null); assert.equal(row.scopedPeakLiveBytes, null); assert.equal(row.cpuMs, null);
    const samples = mode === 'timing' ? contract.warmupBatches + contract.measuredBatches : 1;
    assert.equal(row.batches.length, samples);
    const count = mode === 'timing' ? spec.count : mode === 'smoke' ? 4 : contract.controlInvocationsPerBatch;
    for (const [index, batch] of row.batches.entries()) {
      assert.equal(batch.sample, index); assert.equal(batch.invocations, count);
      assert.equal(batch.warmup, mode === 'timing' && index < contract.warmupBatches);
      assert.ok(Number.isFinite(batch.wallMs) && batch.wallMs >= 0);
    }
    assert.equal(row.completed, 1 + samples * count); assert.equal(row.requestRows.length, row.completed);
    const expected = ['first:0', ...Array.from({ length: samples }, (_, sample) => Array.from({ length: count }, (_, index) => `sample${sample}:${index}`)).flat()].sort();
    assert.deepEqual(row.requestRows.map((item: any) => `${item.batch}:${item.index}`).sort(), expected);
    for (const request of row.requestRows) for (const name of ['wallMs', 'hostDurationMs']) assert.ok(Number.isFinite(request[name]) && request[name] >= 0);
  }
}

export function summarizeInvocationBaseline(reports: any[]) {
  assert.equal(reports.length, contract.processesPerArtifact); reports.forEach(report => validateInvocationCapture(report));
  return { profile: 'woml.invocation-setup-baseline/v1', releaseAccepted: false, primaryAccepted: null,
    allocationBytes: null, scopedPeakLiveBytes: null, cpuMs: null,
    rows: cohorts.map(cohort => {
      const rows = reports.map(report => report.rows.find((row: any) => row.cohort === cohort));
      const processMedians = rows.map(row => median(row.batches.filter((batch: any) => !batch.warmup).map((batch: any) => batch.wallMs)));
      return { cohort, fixtureSha256: rows[0].fixtureSha256, processMedians, batchMedianMs: median(processMedians),
        meanPerInvocationMs: median(processMedians) / fixture(cohort).count,
        readyMs: median(rows.map(row => row.readyMs)), registrationMs: median(rows.map(row => row.registrationMs)),
        firstInvocationMs: median(rows.map(row => row.firstInvocationMs)), teardownMs: median(rows.map(row => row.teardownMs)),
        note: 'Batch mean per invocation, not an individual latency median; concurrent batches include overlapped work' };
    }), releasePrerequisites: contract.releasePrerequisites };
}

export function summarizeInvocationTrace(spans: any[]) {
  assert.ok(spans.length > 0);
  for (const span of spans) {
    assert.equal(span.profile, 'woml.performance-span/v1');
    assert.ok(['host', 'worker'].includes(span.process));
    assert.ok(Number.isFinite(span.durationMs) && span.durationMs >= 0);
    assert.ok(!['source', 'context', 'secrets', 'headers', 'payload'].some(key => key in span));
  }
  return [...new Set(spans.map(span => `${span.process}:${span.name}`))].sort().map(key => {
    const rows = spans.filter(span => `${span.process}:${span.name}` === key);
    return { process: rows[0].process, name: rows[0].name, count: rows.length,
      medianMs: median(rows.map(span => span.durationMs)), maximumMs: Math.max(...rows.map(span => span.durationMs)) };
  });
}

async function build() {
  environment(); baselineSource(); await headroom(); const before = await inputs();
  const output = await directory('invocation-setup-build-');
  const child = Bun.spawn([process.execPath, 'woml-cli/scripts/build-javascript.ts'], { cwd: root, stdout: 'pipe', stderr: 'pipe' });
  const stdout = new Response(child.stdout).text(), stderr = new Response(child.stderr).text(), code = await child.exited;
  await Bun.write(join(output, 'build.log'), `${await stdout}\n${await stderr}`); assert.equal(code, 0, `Build failure retained: ${output}`);
  assert.deepEqual(await inputs(), before); baselineSource();
  const artifacts = [];
  for (const name of files) {
    const path = join(output, name); await copyFile(join(root, 'woml-cli/dist', name), path);
    artifacts.push({ name, path, sha256: sha256(await readFile(path)) });
  }
  for (const file of before.harness) {
    const path = join(output, 'harness', file.path); await mkdir(dirname(path), { recursive: true }); await copyFile(join(root, file.path), path);
  }
  const receipt = { profile: 'woml.invocation-setup-build/v1', mode: 'baseline', createdAt: new Date().toISOString(),
    inputs: before, artifacts, host: host(), toolchain: toolchain(), bunExecutableSha256: sha256(await readFile(process.execPath)),
    nativeArtifact: null, nativeScope: 'Direct built JS host; no native addon loaded or certified', releaseAccepted: false };
  await Bun.write(join(output, 'receipt.json'), JSON.stringify(receipt, null, 2));
  console.log(`ISR build receipt: ${join(output, 'receipt.json')}`);
}

async function capture(receiptPath: string, mode: string) {
  environment(); assert.ok(['timing', 'trace', 'smoke', 'probe'].includes(mode));
  const receipt = JSON.parse(await readFile(resolve(receiptPath), 'utf8'));
  assert.equal(receipt.profile, 'woml.invocation-setup-build/v1'); assert.equal(receipt.mode, 'baseline');
  assert.deepEqual(receipt.host, host()); assert.deepEqual(receipt.toolchain, toolchain());
  assert.equal(receipt.inputs.source.commit, contract.baselineCommit);
  assert.deepEqual(receipt.artifacts.map((file: any) => file.name), files);
  const check = async () => {
    assert.deepEqual(await inputs(), receipt.inputs, 'Source/harness changed; explicit new build required');
    assert.equal(sha256(await readFile(process.execPath)), receipt.bunExecutableSha256);
    for (const file of receipt.artifacts) assert.equal(sha256(await readFile(file.path)), file.sha256);
  };
  await check(); const output = await directory(`invocation-setup-${mode}-`);
  await Bun.write(join(output, 'inputs.json'), JSON.stringify({ receipt, contract, mode, releaseAccepted: false }, null, 2));
  const reports = [];
  for (let processIndex = 0; processIndex < (mode === 'timing' ? contract.processesPerArtifact : 1); processIndex++) {
    await check(); const before = await headroom(), working = join(output, `process${processIndex}`); await mkdir(working);
    const childPath = join(root, 'woml-cli/scripts', mode === 'probe' ? 'invocation-setup-probe.ts' : 'invocation-setup-child.ts');
    const args = mode === 'probe' ? [working] : [receipt.artifacts[0].path, working, mode];
    const child = Bun.spawn([process.execPath, childPath, ...args], { cwd: root, stdout: 'pipe', stderr: 'pipe', timeout: 900_000 });
    const stdout = new Response(child.stdout).text(), stderr = new Response(child.stderr).text(), exitCode = await child.exited;
    await Bun.write(join(working, 'process.log'), `${await stdout}\n${await stderr}`);
    await Bun.write(join(working, 'resources.json'), JSON.stringify({ before, exitCode, usage: resourceRecord(child.resourceUsage()) }, null, 2));
    assert.equal(exitCode, 0, `Failed ISR ${mode} retained: ${working}; do not retry into acceptance`);
    if (mode !== 'probe') {
      const report = JSON.parse(await readFile(join(working, 'capture.json'), 'utf8')); validateInvocationCapture(report, mode); reports.push(report);
      if (mode === 'trace') {
        const traces = [];
        for (const cohort of cohorts) {
          const spans = (await readFile(join(working, `${cohort}.ndjson`), 'utf8')).trim().split('\n').map(line => JSON.parse(line));
          assert.ok(spans.some(span => span.name === 'host.invoke_worker'));
          assert.equal(spans.filter(span => span.name === 'worker.execute_invocation').length, 1 + contract.controlInvocationsPerBatch);
          traces.push({ cohort, rows: summarizeInvocationTrace(spans) });
        }
        await Bun.write(join(output, 'trace-summary.json'), JSON.stringify({ traces, releaseAccepted: false, note: 'Profiler-enabled diagnosis. Nested spans overlap; never subtract clocks across realms or use traced timings for acceptance.' }, null, 2));
      }
    }
    await check(); console.log(`ISR ${mode}: process ${processIndex + 1}/${mode === 'timing' ? contract.processesPerArtifact : 1}`);
  }
  if (mode === 'timing') await Bun.write(join(output, 'summary.json'), JSON.stringify(summarizeInvocationBaseline(reports), null, 2));
  console.log(`ISR ${mode} evidence: ${output}`);
}

if (import.meta.main) {
  const [command, flag, path] = process.argv.slice(2);
  if (command === 'build' && flag === undefined) await build();
  else { assert.ok(flag === '--receipt' && path && process.argv.length === 5, 'Use build OR timing|trace|probe|smoke --receipt PATH'); await capture(path, command!); }
}
