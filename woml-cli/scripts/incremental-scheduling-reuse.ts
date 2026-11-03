#!/usr/bin/env bun
// ISQ2/3 attribution only. Explicit receipts; no implicit builds or retries.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile } from 'node:fs/promises';
import { arch, cpus, hostname, platform } from 'node:os';
import { join, resolve } from 'node:path';
import { headroom, resourceRecord, snapshot } from './validated-state-baseline';
import { median } from './validated-state-comparison';

const root = resolve(import.meta.dir, '../..');
const sizes = [1, 4, 16, 64, 256, 1024];
const sha = (bytes: string | Uint8Array) => createHash('sha256').update(bytes).digest('hex');
type Capture = { resources: boolean; rows: Record<string, any>[] };

export function validateReuseProbe(value: unknown, resources: boolean): asserts value is Capture {
  const report = value as Record<string, any>;
  assert.equal(report.profile, 'woml.isq-reuse-probe/v1');
  assert.equal(report.resources, resources); assert.equal(report.releaseAccepted, false);
  assert.equal(report.rows.length, sizes.length * 4);
  const fixtures = new Map<number, string>();
  const clock = (time: any) => {
    assert.ok(time && Number.isFinite(time.wallMs) && time.wallMs >= 0);
    assert.ok(time.threadCpuMs === null || Number.isFinite(time.threadCpuMs) && time.threadCpuMs >= 0);
  };
  const allocation = (usage: any) => {
    assert.ok(usage && ['allocated', 'freed', 'live', 'peak'].every(name => Number.isSafeInteger(usage[name])));
    assert.ok(usage.allocated >= 0 && usage.freed >= 0 && usage.peak >= 0);
    assert.equal(usage.live, usage.allocated - usage.freed);
  };
  for (const [index, row] of report.rows.entries()) {
    assert.equal(row.nodes, sizes[Math.floor(index / 4)]); assert.equal(row.events, row.nodes * 2);
    assert.equal(row.sample, index % 4); assert.equal(row.warmup, row.sample === 0);
    assert.match(row.fixtureSha256, /^[0-9a-f]{64}$/);
    if (fixtures.has(row.nodes)) assert.equal(row.fixtureSha256, fixtures.get(row.nodes));
    fixtures.set(row.nodes, row.fixtureSha256);
    assert.ok(Number.isSafeInteger(row.engineStackBytes) && row.engineStackBytes > 0);
    for (const operation of ['startup', 'firstQuery', 'warmQueryBatch20', 'definitionCloneBatch20']) {
      if (resources) assert.equal(row[operation], null); else clock(row[operation]);
    }
    if (resources) { allocation(row.firstQueryAllocation); allocation(row.history.allocation); }
    else { assert.equal(row.firstQueryAllocation, null); clock(row.history); }
  }
}

export function summarizeReuseProbe(before: Capture[], after: Capture[]) {
  assert.equal(before.length, 7); assert.equal(after.length, 7);
  for (const report of [...before, ...after]) {
    validateReuseProbe(report, false);
    assert.deepEqual(report.rows.map(row => row.fixtureSha256), before[0]!.rows.map(row => row.fixtureSha256));
  }
  return sizes.flatMap(nodes => ['startup', 'firstQuery', 'warmQueryBatch20', 'definitionCloneBatch20', 'history'].flatMap(operation => ['wallMs', 'threadCpuMs'].map(metric => {
    const samples = (reports: Capture[]) => reports.map(report => {
      const values = report.rows.filter(row => row.nodes === nodes && !row.warmup).map(row => row[operation][metric]);
      return values.includes(null) ? null : median(values) / (operation.endsWith('Batch20') ? 20 : 1);
    });
    const a = samples(before), b = samples(after);
    const baseline = a.includes(null) ? null : median(a as number[]), candidate = b.includes(null) ? null : median(b as number[]);
    return { nodes, operation, metric, baselineProcesses: a, candidateProcesses: b, baseline, candidate,
      improvementPercent: baseline === null || candidate === null || baseline === 0 ? null : (baseline - candidate) / baseline * 100 };
  })));
}

async function main() {
  const args = process.argv.slice(2);
  assert.ok((args.length === 2 && args[0] === '--receipt') || (args.length === 4 && args[0] === '--baseline' && args[2] === '--candidate'),
    'Use --receipt BASELINE before edits, or --baseline BASELINE --candidate CANDIDATE');
  for (const name of ['WOML_PROFILE', 'WOML_PROFILE_OUTPUT', 'WOML_ISQ_REUSE_CAPTURE', 'WOML_ISQ_REUSE_RESOURCES', 'LD_PRELOAD', 'DYLD_INSERT_LIBRARIES', 'RUST_MIN_STACK']) assert.ok(!process.env[name]);
  const receiptPaths = args.length === 2 ? [args[1]!] : [args[1]!, args[3]!];
  const receipts: any[] = await Promise.all(receiptPaths.map(async path => JSON.parse(await readFile(resolve(path), 'utf8'))));
  const harness = await readFile(import.meta.path);
  for (const receipt of receipts) {
    assert.equal(receipt.profile, 'woml.validated-state-build/v1'); assert.equal(receipt.buildMode, 'release-unit-test');
    assert.deepEqual(receipt.host, { platform: platform(), arch: arch(), hostname: hostname(), cpu: cpus()[0]?.model });
    assert.equal(receipt.toolchain.bun, Bun.version);
    for (const name of ['rustc', 'cargo']) {
      const result = Bun.spawnSync([name, '--version'], { stdout: 'pipe', stderr: 'pipe' });
      assert.equal(result.exitCode, 0); assert.equal(receipt.toolchain[name], result.stdout.toString().trim());
    }
    assert.equal(sha(JSON.stringify(receipt.source.files)), receipt.source.sha256);
  }
  const check = async () => {
    assert.deepEqual(await snapshot(), receipts.at(-1).source);
    assert.equal(sha(await readFile(import.meta.path)), sha(harness));
    for (const receipt of receipts) assert.equal(sha(await readFile(receipt.binary)), receipt.binarySha256);
  };
  await check();
  const parent = join(root, 'woml-cli/.woml/performance'); await mkdir(parent, { recursive: true });
  const directory = await mkdtemp(join(parent, 'isq-reuse-'));
  await Bun.write(join(directory, 'inputs.json'), JSON.stringify({ receipts, harnessSha256: sha(harness), protocol: {
    sizes, warmups: 1, samples: 3, queryBatch: 20, pairs: 7, order: 'alternating-artifacts', statistic: 'median-of-process-medians',
    memory: 'System GlobalAlloc requested bytes in separate single-thread scopes; not native/SQLite allocation, RSS or realloc transient overlap',
    acceptance: 'Attribution diagnostic only; original ISQ0 acceptance, controls and release blockers remain unchanged',
  } }, null, 2));
  await Bun.write(join(directory, 'harness.ts'), harness);
  const cohorts: Capture[][] = receipts.map(() => []);
  for (const resources of [false, true]) {
    for (let pair = 0; pair < (resources ? 1 : 7); pair++) {
      const order: number[] = receipts.length === 2 && pair % 2 ? [1, 0] : receipts.map((_, index) => index);
      for (const index of order) {
        await check(); const before = await headroom();
        const path = join(directory, `${resources ? 'resource' : 'timing'}-${pair}-${index}.json`);
        const child = Bun.spawn([receipts[index].binary, '--exact', 'engine::readiness::reuse_tests::capture_reuse', '--ignored', '--nocapture', '--test-threads=1'], {
          cwd: root, env: { ...process.env, WOML_ISQ_REUSE_CAPTURE: path, ...(resources ? { WOML_ISQ_REUSE_RESOURCES: '1' } : {}) }, stdout: 'pipe', stderr: 'pipe', timeout: 300_000 });
        const stdout = new Response(child.stdout).text(), stderr = new Response(child.stderr).text(), exitCode = await child.exited;
        await Bun.write(path + '.log', `${await stdout}\n${await stderr}`);
        await Bun.write(path + '.resources.json', JSON.stringify({ before, exitCode, usage: resourceRecord(child.resourceUsage()) }, null, 2));
        assert.equal(exitCode, 0, `Failed capture retained in ${directory}`);
        const report = JSON.parse(await readFile(path, 'utf8')); validateReuseProbe(report, resources);
        if (!resources) cohorts[index]!.push(report);
        await check(); console.log(`ISQ reuse ${resources ? 'resource' : pair + 1}: artifact ${index}`);
      }
    }
  }
  const rows = receipts.length === 2 ? summarizeReuseProbe(cohorts[0]!, cohorts[1]!) : null;
  await Bun.write(join(directory, 'summary.json'), JSON.stringify({ rows, releaseAccepted: false }, null, 2));
  console.log(`ISQ reuse capture: ${directory}`);
}
if (import.meta.main) await main();
