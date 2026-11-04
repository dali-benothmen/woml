#!/usr/bin/env bun
// Explicit capture only. No builds, installs, retries, trimming, or budget edits.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile } from 'node:fs/promises';
import { arch, cpus, hostname, platform } from 'node:os';
import { join, resolve } from 'node:path';
import acceptance from '../../docs/validated-state-reuse-acceptance.v1.json';
import { headroom, resourceRecord, snapshot, validateCapture } from './validated-state-baseline';

const root = resolve(import.meta.dir, '../..');
const hash = (value: string | Uint8Array) => createHash('sha256').update(value).digest('hex');
const testName = 'durable::boundary_tests::validated_state_baseline::capture';
export function linuxMaximumRssBytes(value: string) {
  assert.match(value.trim(), /^[0-9]+$/);
  const bytes = Number(value.trim()) * 1024;
  assert.ok(Number.isSafeInteger(bytes) && bytes > 0, 'GNU time Linux maximum RSS must be positive KiB');
  return bytes;
}
function git(...args: string[]) {
  const result = Bun.spawnSync(['git', ...args], { cwd: root, stdout: 'pipe', stderr: 'pipe' });
  assert.equal(result.exitCode, 0, result.stderr.toString());
  return result.stdout;
}
export function median(values: number[]) {
  assert.ok(values.length && values.every(value => Number.isFinite(value) && value >= 0));
  const sorted = [...values].sort((a, b) => a - b), middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[middle]! : (sorted[middle - 1]! + sorted[middle]!) / 2;
}
type Capture = { rows: Record<string, any>[] };
export function summarizePair(baseline: Capture[], candidate: Capture[]) {
  assert.equal(baseline.length, acceptance.processesPerArtifact);
  assert.equal(candidate.length, acceptance.processesPerArtifact);
  [...baseline, ...candidate].forEach(validateCapture);
  for (const capture of [...baseline, ...candidate]) {
    assert.deepEqual(capture.rows.map(row => row.fixtureSha256), baseline[0]!.rows.map(row => row.fixtureSha256), 'Baseline/candidate fixture substitution');
  }
  const rows = acceptance.eventLengths.flatMap(events => ['miss', 'hits', 'structuralFold'].flatMap(operation => ['threadCpuMs', 'wallMs'].map(metric => {
    const processMedians = (cohort: Capture[]) => cohort.map(capture => {
      const values = capture.rows.filter(row => row.events === events && !row.warmup)
        .flatMap(row => operation === 'hits' ? row.hits.map((hit: any) => hit[metric]) : [row[operation][metric]]);
      return values.some(value => value === null) ? null : median(values);
    });
    const before = processMedians(baseline), after = processMedians(candidate);
    const b = before.includes(null) ? null : median(before as number[]);
    const c = after.includes(null) ? null : median(after as number[]);
    return { events, operation, metric, baselineProcessMedians: before, candidateProcessMedians: after,
      baseline: b, candidate: c, improvementPercent: b === null || c === null || b === 0 ? null : (b - c) / b * 100 };
  })));
  const primary = rows.find(row => row.events === acceptance.primary.events && row.operation === 'miss' && row.metric === acceptance.primary.metric)!;
  const investigations = rows.filter(row => row.improvementPercent !== null && row.improvementPercent < -acceptance.investigation.controlMedianRegressionPercent);
  return { statistic: acceptance.statistic, rows, primary,
    primaryAccepted: primary.improvementPercent !== null && primary.improvementPercent >= acceptance.primary.minimumImprovementPercent,
    investigations, allocationMeasurement: null, peakLiveBytes: null,
    releaseAccepted: false,
    limitations: ['Engine cache-miss/hit and isolated fold only; not native workflow latency.',
      'Uninstrumented timing; allocations and scoped peak-live bytes need separate capture.',
      'A primary pass alone does not close native/package/platform or resource gates.'] };
}

export async function verifyHistoricalSource(receipt: any, baselineCommit: string) {
  assert.match(baselineCommit, /^[0-9a-f]{40}$/);
  assert.equal(hash(JSON.stringify(receipt.source.files)), receipt.source.sha256);
  assert.ok(Array.isArray(receipt.source.files) && receipt.source.files.length > 900);
  assert.equal(new Set(receipt.source.files.map((file: any) => file.path)).size, receipt.source.files.length);
  const manifest = new Map(receipt.source.files.map((file: any) => [file.path, file.sha256]));
  // A missing source file must not be hidden by removing it from the receipt.
  const paths = git('ls-tree', '-r', '--name-only', baselineCommit, '--', 'core', 'woml/src', 'woml/tests/fixtures', 'woml-cli/src').toString().trim().split('\n');
  for (const path of paths) assert.ok(manifest.has(path), `Missing baseline source: ${path}`);
  for (const file of receipt.source.files) {
    assert.equal(hash(git('show', `${baselineCommit}:${file.path}`)), file.sha256, `Historical source mismatch: ${file.path}`);
  }
}

export async function artifactPairCheck(baseline: any, candidate: any, baselineCommit: string) {
  await verifyHistoricalSource(baseline, baselineCommit);
  assert.notEqual(baseline.binarySha256, candidate.binarySha256, 'Identical binaries cannot establish candidate benefit');
  const version = (name: string) => {
    const result = Bun.spawnSync([name, '--version'], { stdout: 'pipe', stderr: 'pipe' });
    assert.equal(result.exitCode, 0); return result.stdout.toString().trim();
  };
  return async () => {
    assert.deepEqual(await snapshot(), candidate.source, 'Candidate source changed; build an explicit fresh receipt');
    for (const receipt of [baseline, candidate]) {
      assert.equal(receipt.profile, 'woml.validated-state-build/v1');
      assert.equal(receipt.buildMode, 'release-unit-test');
      assert.deepEqual(receipt.toolchain, { rustc: version('rustc'), cargo: version('cargo'), bun: Bun.version });
      assert.deepEqual(receipt.host, { platform: platform(), arch: arch(), hostname: hostname(), cpu: cpus()[0]?.model });
      assert.equal(hash(await readFile(receipt.binary)), receipt.binarySha256, 'Binary substitution');
    }
  };
}

async function main() {
  const args = process.argv.slice(2);
  assert.ok(args.length === 6 && args[0] === '--baseline' && args[2] === '--candidate' && args[4] === '--baseline-commit',
    'Use --baseline RECEIPT --candidate RECEIPT --baseline-commit FULL_SHA');
  assert.ok(!process.env.WOML_PROFILE && !process.env.WOML_PROFILE_OUTPUT && !process.env.WOML_VSR_CAPTURE, 'Unset tracing/capture variables');
  assert.ok(!process.env.RUST_MIN_STACK && !process.env.LD_PRELOAD && !process.env.DYLD_INSERT_LIBRARIES,
    'Use ordinary test stacks and no injected profiler/allocator for timing');
  const baseline = JSON.parse(await readFile(resolve(args[1]!), 'utf8'));
  const candidate = JSON.parse(await readFile(resolve(args[3]!), 'utf8'));
  const harness = await readFile(import.meta.path);
  // Linux GNU time documents %M in KiB. Unlike Bun's unverified runtime RSS
  // unit, this can enter the byte-based whole-process investigation threshold.
  // It is NOT scoped peak-live allocation or Rust-only memory.
  let observer: { path: string; sha256: string; version: string; metric: string } | null = null;
  if (process.platform === 'linux' && await Bun.file('/usr/bin/time').exists()) {
    const version = Bun.spawnSync(['/usr/bin/time', '--version'], { stdout: 'pipe', stderr: 'pipe' });
    assert.equal(version.exitCode, 0); assert.ok(version.stdout.toString().includes('(GNU Time)'));
    observer = { path: '/usr/bin/time', sha256: hash(await readFile('/usr/bin/time')), version: version.stdout.toString().trim(), metric: 'Linux GNU time %M times 1024 bytes; entire test process including fixture setup' };
  }
  const artifacts = await artifactPairCheck(baseline, candidate, args[5]!);
  const check = async () => {
    assert.equal(hash(await readFile(import.meta.path)), hash(harness), 'Comparison harness changed during capture');
    if (observer) assert.equal(hash(await readFile(observer.path)), observer.sha256, 'Resource observer changed');
    await artifacts();
  };
  await check(); await headroom();
  const parent = join(root, 'woml-cli/.woml/performance'); await mkdir(parent, { recursive: true });
  const directory = await mkdtemp(join(parent, 'validated-state-comparison-'));
  const provenance = { baselineCommit: args[5], frozenBaselineCommit: acceptance.baselineCommit,
    baselineMatchesFrozen: args[5] === acceptance.baselineCommit,
    comparisonKind: args[5] === acceptance.baselineCommit ? 'frozen-baseline' : 'explicit-attribution-baseline-not-release-acceptance' };
  await Bun.write(join(directory, 'comparison-harness.ts'), harness);
  await Bun.write(join(directory, 'inputs.json'), JSON.stringify({ baseline, baselineCommit: args[5], candidate, acceptance, observer, comparisonHarnessSha256: hash(harness) }, null, 2));
  const reports: Record<'baseline' | 'candidate', Capture[]> = { baseline: [], candidate: [] };
  const rss: Record<'baseline' | 'candidate', number[]> = { baseline: [], candidate: [] };
  for (let pair = 0; pair < acceptance.processesPerArtifact; pair++) {
    // Keep all seven pairs, alternating B/C then C/B, without opportunistic reruns.
    const order = pair % 2 === 0 ? ['baseline', 'candidate'] as const : ['candidate', 'baseline'] as const;
    for (const label of order) {
      await check(); const resources = await headroom();
      const output = join(directory, `${pair}-${label}.json`);
      const rssOutput = join(directory, `${pair}-${label}-rss-kib.txt`);
      const executable = [(label === 'baseline' ? baseline : candidate).binary, '--exact', testName, '--ignored', '--nocapture', '--test-threads=1'];
      const child = Bun.spawn(observer ? [observer.path, '-f', '%M', '-o', rssOutput, ...executable] : executable, {
        cwd: root, env: { ...process.env, WOML_VSR_CAPTURE: output }, stdout: 'pipe', stderr: 'pipe' });
      const stdout = new Response(child.stdout).text(), stderr = new Response(child.stderr).text();
      const exitCode = await child.exited;
      await Bun.write(join(directory, `${pair}-${label}.log`), `${await stdout}\n${await stderr}`);
      await Bun.write(join(directory, `${pair}-${label}-resources.json`), JSON.stringify({ pair, label, order, before: resources, exitCode,
        usageScope: observer ? 'GNU time supervisor and child; not scoped Rust thread CPU' : 'test process', usage: resourceRecord(child.resourceUsage()) }, null, 2));
      assert.equal(exitCode, 0, `Failed capture; all raw evidence retained: ${directory}`);
      if (observer) rss[label].push(linuxMaximumRssBytes(await readFile(rssOutput, 'utf8')));
      const report = JSON.parse(await readFile(output, 'utf8')); validateCapture(report);
      reports[label].push(report); await check();
      console.log(`Pair ${pair + 1}/${acceptance.processesPerArtifact}: ${label} retained`);
    }
  }
  const maximumRss = observer ? { baselineProcessBytes: rss.baseline, candidateProcessBytes: rss.candidate,
    baselineMedianBytes: median(rss.baseline), candidateMedianBytes: median(rss.candidate),
    medianIncreaseBytes: median(rss.candidate) - median(rss.baseline),
    requiresInvestigation: median(rss.candidate) - median(rss.baseline) > acceptance.investigation.ownRssOrPeakLiveIncreaseBytes } : null;
  const report = { ...provenance, ...summarizePair(reports.baseline, reports.candidate), wholeProcessMaximumRss: maximumRss };
  await Bun.write(join(directory, 'summary.json'), JSON.stringify(report, null, 2));
  console.log(JSON.stringify({ directory, ...provenance, primary: report.primary, primaryAccepted: report.primaryAccepted,
    investigations: report.investigations, wholeProcessMaximumRss: maximumRss, releaseAccepted: false }, null, 2));
  assert.ok(report.primaryAccepted, `Frozen primary target NOT met; see ${directory}. Do not raise the threshold or attribute readiness-index gains to reuse.`);
}
if (import.meta.main) await main();
