#!/usr/bin/env bun
// Separate diagnosis, NOT a replacement cohort or an acceptance-gate rerun.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { headroom, snapshot } from './validated-state-baseline';
import { median } from './validated-state-comparison';

const root = resolve(import.meta.dir, '../..');
const hash = (bytes: Uint8Array) => createHash('sha256').update(bytes).digest('hex');
assert.ok(process.argv.length === 4 && process.argv[2] === '--receipt', 'Use an explicit optimized engine build receipt');
for (const name of ['WOML_PROFILE', 'WOML_PROFILE_OUTPUT', 'WOML_ISQ_CONTROL_MODE', 'WOML_ISQ_CONTROL_CAPTURE', 'LD_PRELOAD', 'DYLD_INSERT_LIBRARIES', 'RUST_MIN_STACK']) assert.ok(!process.env[name]);
const receipt = JSON.parse(await readFile(resolve(process.argv[3]!), 'utf8'));
assert.equal(receipt.profile, 'woml.validated-state-build/v1');
assert.equal(receipt.buildMode, 'release-unit-test');
const harness = await readFile(import.meta.path);
const check = async () => {
  assert.deepEqual(await snapshot(), receipt.source);
  assert.equal(hash(await readFile(receipt.binary)), receipt.binarySha256);
  assert.equal(hash(await readFile(import.meta.path)), hash(harness));
};
await check();
const parent = join(root, 'woml-cli/.woml/performance'); await mkdir(parent, { recursive: true });
const directory = await mkdtemp(join(parent, 'isq-control-diagnosis-'));
await Bun.write(join(directory, 'inputs.json'), JSON.stringify({ receipt, harnessSha256: hash(harness), processesPerMode: 7, order: 'alternating-full-membership',
  purpose: 'Identical full-query/topology functions in one binary, before and after four validations differing only in membership strategy; diagnosis, not acceptance.' }, null, 2));
await Bun.write(join(directory, 'harness.ts'), harness);
const reports: Record<string, any[]> = { full: [], membership: [] };
for (let pair = 0; pair < 7; pair++) {
  for (const mode of pair % 2 === 0 ? ['full', 'membership'] : ['membership', 'full']) {
    await check(); await headroom();
    const output = join(directory, `${pair}-${mode}.json`);
    const child = Bun.spawn([receipt.binary, '--exact', 'engine::readiness::incremental::capture_controls', '--ignored', '--nocapture', '--test-threads=1'], {
      cwd: root, env: { ...process.env, WOML_ISQ_CONTROL_MODE: mode, WOML_ISQ_CONTROL_CAPTURE: output }, stdout: 'pipe', stderr: 'pipe', timeout: 300_000 });
    const stdout = new Response(child.stdout).text(), stderr = new Response(child.stderr).text(), exitCode = await child.exited;
    await Bun.write(join(directory, `${pair}-${mode}.log`), `${await stdout}\n${await stderr}`);
    assert.equal(exitCode, 0, `Failed diagnosis retained at ${directory}`);
    const report = JSON.parse(await readFile(output, 'utf8'));
    assert.equal(report.profile, 'woml.isq-controls-diagnostic/v1'); assert.equal(report.mode, mode);
    assert.equal(report.rows.length, 12); assert.equal(report.releaseAccepted, false);
    assert.match(report.fixtureSha256, /^[a-f0-9]{64}$/);
    for (const [index, row] of report.rows.entries()) {
      assert.equal(row.stage, index < 6 ? 'before-history' : 'after-history');
      assert.equal(row.sample, index % 6); assert.equal(row.warmup, row.sample === 0);
      for (const operation of ['fullQueryBatch100', 'topologyBatch100']) for (const metric of ['threadCpuMs', 'wallMs']) {
        const value = row[operation][metric];
        assert.ok(metric === 'threadCpuMs' && value === null || Number.isFinite(value) && value >= 0);
      }
    }
    reports[mode]!.push(report); await check(); console.log(`Control diagnosis ${pair + 1}/7: ${mode}`);
  }
}
assert.equal(new Set(Object.values(reports).flat().map(report => report.fixtureSha256)).size, 1);
const rows = ['before-history', 'after-history'].flatMap(stage => ['fullQueryBatch100', 'topologyBatch100'].flatMap(operation => ['threadCpuMs', 'wallMs'].map(metric => {
  const processMedians = (mode: string) => reports[mode]!.map(report => {
    const values = report.rows.filter((row: any) => row.stage === stage && !row.warmup).map((row: any) => row[operation][metric]);
    return values.includes(null) ? null : median(values) / 100;
  });
  const full = processMedians('full'), membership = processMedians('membership');
  const before = full.includes(null) ? null : median(full as number[]), after = membership.includes(null) ? null : median(membership as number[]);
  return { stage, operation, metric, fullProcessMedians: full, membershipProcessMedians: membership, full: before, membership: after,
    differencePercent: before === null || after === null || before === 0 ? null : (after - before) / before * 100 };
})));
await Bun.write(join(directory, 'summary.json'), JSON.stringify({ rows, releaseAccepted: false }, null, 2));
console.log(`Control diagnosis retained: ${directory}`);
