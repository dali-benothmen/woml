#!/usr/bin/env bun
// Separate diagnostic processes. Instrumented timings NEVER enter acceptance.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import acceptance from '../../docs/validated-state-reuse-acceptance.v1.json';
import { headroom, validateCapture } from './validated-state-baseline';
import { artifactPairCheck, median } from './validated-state-comparison';

const root = resolve(import.meta.dir, '../..');
const hash = (bytes: Uint8Array) => createHash('sha256').update(bytes).digest('hex');
export function parseMemusage(text: string) {
  const plain = text.replace(/\x1b\[[0-9;]*m/g, '');
  const matches = [...plain.matchAll(/Memory usage summary: heap total:\s*(\d+), heap peak:\s*(\d+), stack peak:\s*(\d+)/g)];
  assert.equal(matches.length, 1, 'Require one profiler summary for the exact test process');
  const numbers = matches[0]!.slice(1).map(Number);
  assert.ok(numbers.every(value => Number.isSafeInteger(value) && value >= 0));
  assert.ok(numbers[0]! > 0 && numbers[1]! > 0, 'Missing allocator observation is not zero');
  return { interceptedHeapTotalBytes: numbers[0]!, reportedHeapPeakBytes: numbers[1]!,
    reportedStackPeakBytes: numbers[2]! };
}
async function main() {
  const args = process.argv.slice(2);
  assert.ok(args.length === 6 && args[0] === '--baseline' && args[2] === '--candidate' && args[4] === '--baseline-commit',
    'Use --baseline RECEIPT --candidate RECEIPT --baseline-commit FULL_SHA');
  assert.equal(process.platform, 'linux'); assert.equal(process.arch, 'x64', 'This explicit glibc probe currently supports Linux x64 only');
  for (const name of ['LD_PRELOAD', 'WOML_PROFILE', 'WOML_PROFILE_OUTPUT', 'WOML_VSR_CAPTURE', 'RUST_MIN_STACK']) assert.ok(!process.env[name], `Unset ${name}`);
  const library = '/usr/lib/x86_64-linux-gnu/libmemusage.so';
  const libraryHash = hash(await readFile(library));
  const baseline = JSON.parse(await readFile(resolve(args[1]!), 'utf8'));
  const candidate = JSON.parse(await readFile(resolve(args[3]!), 'utf8'));
  const artifacts = await artifactPairCheck(baseline, candidate, args[5]!);
  const harness = await readFile(import.meta.path);
  const comparisonHarness = await readFile(resolve(import.meta.dir, 'validated-state-comparison.ts'));
  const check = async () => {
    await artifacts();
    assert.equal(hash(await readFile(library)), libraryHash);
    assert.equal(hash(await readFile(import.meta.path)), hash(harness));
    assert.equal(hash(await readFile(resolve(import.meta.dir, 'validated-state-comparison.ts'))), hash(comparisonHarness));
  };
  await check(); await headroom();
  const parent = join(root, 'woml-cli/.woml/performance'); await mkdir(parent, { recursive: true });
  const directory = await mkdtemp(join(parent, 'validated-state-allocations-'));
  await Bun.write(join(directory, 'allocation-harness.ts'), harness);
  await Bun.write(join(directory, 'comparison-harness.ts'), comparisonHarness);
  await Bun.write(join(directory, 'inputs.json'), JSON.stringify({ baseline, candidate, baselineCommit: args[5], acceptance,
    library, libraryHash, harnessSha256: hash(harness), comparisonHarnessSha256: hash(comparisonHarness),
    scope: 'glibc malloc/calloc/realloc/free interception across the entire capture process, including fixture setup and warmups; no mmap/aligned-allocation claim; not per-cache-miss attribution' }, null, 2));
  const samples = { baseline: [] as ReturnType<typeof parseMemusage>[], candidate: [] as ReturnType<typeof parseMemusage>[] };
  let identity: string[] | undefined;
  for (let pair = 0; pair < acceptance.processesPerArtifact; pair++) {
    for (const label of (pair % 2 ? ['candidate', 'baseline'] : ['baseline', 'candidate']) as ('baseline' | 'candidate')[]) {
      await check(); const before = await headroom();
      const output = join(directory, `${pair}-${label}.json`);
      const child = Bun.spawn([(label === 'baseline' ? baseline : candidate).binary, '--exact',
        'durable::boundary_tests::validated_state_baseline::capture', '--ignored', '--nocapture', '--test-threads=1'], {
        cwd: root, stdout: 'pipe', stderr: 'pipe', env: { ...process.env, LD_PRELOAD: library,
          MEMUSAGE_NO_TIMER: 'yes', MEMUSAGE_OUTPUT: undefined, MEMUSAGE_PROG_NAME: undefined,
          MEMUSAGE_TRACE_MMAP: undefined, WOML_VSR_CAPTURE: output } });
      const stdout = new Response(child.stdout).text(), stderr = new Response(child.stderr).text();
      const exitCode = await child.exited;
      const diagnostics = await stderr;
      await Bun.write(join(directory, `${pair}-${label}.log`), `${await stdout}\n${diagnostics}`);
      await Bun.write(join(directory, `${pair}-${label}-resources.json`), JSON.stringify({ before, exitCode, instrumentedTimingsRejected: true }, null, 2));
      assert.equal(exitCode, 0, `Allocation diagnosis failed; retained ${directory}`);
      const capture = JSON.parse(await readFile(output, 'utf8')); validateCapture(capture);
      const current = capture.rows.map((row: any) => row.fixtureSha256);
      if (identity) assert.deepEqual(current, identity); else identity = current;
      samples[label].push(parseMemusage(diagnostics)); await check();
      console.log(`Resource pair ${pair + 1}/${acceptance.processesPerArtifact}: ${label} retained`);
    }
  }
  const total = (label: 'baseline' | 'candidate') => median(samples[label].map(sample => sample.interceptedHeapTotalBytes));
  const peak = (label: 'baseline' | 'candidate') => median(samples[label].map(sample => sample.reportedHeapPeakBytes));
  const increasePercent = (total('candidate') - total('baseline')) / total('baseline') * 100;
  const report = { profile: 'woml.validated-state-allocation-diagnosis/v1', samples,
    baselineHeapTotalMedianBytes: total('baseline'), candidateHeapTotalMedianBytes: total('candidate'), heapTotalIncreasePercent: increasePercent,
    baselineReportedHeapPeakMedianBytes: peak('baseline'), candidateReportedHeapPeakMedianBytes: peak('candidate'),
    reportedHeapPeakMedianIncreaseBytes: peak('candidate') - peak('baseline'),
    requiresAllocationInvestigation: increasePercent > acceptance.investigation.allocatedBytesIncreasePercent,
    scope: 'Whole process profiler observations, not scoped cache-miss allocations or total process RSS. Heap total counts intercepted requests and realloc growth; reported peaks retain profiler semantics.',
    instrumentedTimingsRejected: true, scopedAllocationMeasurement: null, scopedPeakLiveBytes: null, releaseAccepted: false };
  await Bun.write(join(directory, 'summary.json'), JSON.stringify(report, null, 2));
  console.log(JSON.stringify({ directory, ...report }, null, 2));
}
if (import.meta.main) await main();
