#!/usr/bin/env bun
// Explicit release artifacts only. This command never builds, installs, or retries a sample.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { copyFile, mkdir, mkdtemp, readFile, statfs } from 'node:fs/promises';
import { cpus, loadavg } from 'node:os';
import { join, resolve } from 'node:path';
import { startupAcceptance, startupScenarios, type StartupScenario } from './startup-separation-baseline';
import { sessionFixture, type SessionResult } from './runtime-session-profile-child';
import { summarizeSamples } from './performance-measurement';

export const comparisonProfile = 'woml.startup-separation-comparison/v1';
export type Variant = 'baseline' | 'candidate';
export interface Observation { variant: Variant; scenario: StartupScenario; batch: number; result: SessionResult }
export function comparisonSchedule() {
  return Array.from({ length: startupAcceptance.batches }, (_, batch) => {
    const scenarios = batch % 2 ? startupScenarios().reverse() : startupScenarios();
    return scenarios.flatMap((scenario, index) => {
      const variants: Variant[] = (batch + index) % 2 ? ['candidate', 'baseline'] : ['baseline', 'candidate'];
      return variants.map(variant => ({ variant, scenario, batch }));
    });
  }).flat();
}
function finite(value: number): number { assert.ok(Number.isFinite(value) && value >= 0); return value; }
const median = (values: number[]) => summarizeSamples(values.map(finite)).medianMs;
export function validateObservation(row: Observation): void {
  const expected = startupScenarios().find(scenario => scenario.id === row.scenario.id);
  assert.deepEqual(row.scenario, expected, 'Unreviewed scenario');
  assert.ok(row.variant === 'baseline' || row.variant === 'candidate');
  assert.ok(Number.isSafeInteger(row.batch) && row.batch >= 0 && row.batch < startupAcceptance.batches);
  const result = row.result;
  assert.equal(result.profile, 'woml.runtime-session-sample/v1');
  assert.equal(result.case, row.scenario.case); assert.equal(result.mode, row.scenario.mode);
  assert.deepEqual(result.parameters, { retained: row.scenario.retained, concurrency: row.scenario.concurrency, idleMs: row.scenario.idleMs });
  assert.equal(result.samples.length, startupAcceptance.runsPerProcess);
  assert.equal(new Set(result.samples.map(sample => sample.runId)).size, result.samples.length);
  result.samples.forEach((sample, index) => {
    assert.equal(sample.index, index); assert.ok(finite(sample.durationMs) > 0);
    finite(sample.ownProcessCpuMs); assert.ok(finite(sample.ownProcessRssBytes!) > 0);
    assert.ok(Number.isSafeInteger(sample.eventCount) && sample.eventCount! > 0);
  });
  if (row.scenario.mode === 'activated') {
    finite(result.prepareMs!); finite(result.activateMs!); finite(result.shutdownMs!);
  }
}

export function compareObservations(observations: readonly Observation[]) {
  assert.equal(observations.length, comparisonSchedule().length, 'Incomplete cohort cannot pass');
  const keys = observations.map(row => `${row.batch}/${row.scenario.id}/${row.variant}`);
  assert.equal(new Set(keys).size, keys.length, 'Duplicate batch cannot replace an independent process');
  observations.forEach(validateObservation);
  const metrics = [
    ['firstMs', (r: SessionResult) => r.samples[0]!.durationMs],
    ['laterMs', (r: SessionResult) => median(r.samples.slice(1).map(s => s.durationMs))],
    ['firstCpuMs', (r: SessionResult) => r.samples[0]!.ownProcessCpuMs],
    ['laterCpuMs', (r: SessionResult) => median(r.samples.slice(1).map(s => s.ownProcessCpuMs))],
    ['rssBytes', (r: SessionResult) => Math.max(...r.samples.map(s => s.ownProcessRssBytes!))],
    ['prepareMs', (r: SessionResult) => r.prepareMs!],
    ['activateMs', (r: SessionResult) => r.activateMs!],
    ['shutdownMs', (r: SessionResult) => r.shutdownMs!],
  ] as const;
  const rows = startupScenarios().map(scenario => {
    const pairs = Array.from({ length: startupAcceptance.batches }, (_, batch) => {
      const find = (variant: Variant) => observations.find(row => row.variant === variant && row.batch === batch && row.scenario.id === scenario.id)!.result;
      const baseline = find('baseline'), candidate = find('candidate');
      assert.equal(candidate.definitionHash, baseline.definitionHash, 'Definition identity drift');
      assert.equal(candidate.modelVersion, baseline.modelVersion, 'Model version drift');
      assert.deepEqual(candidate.samples.map(s => s.eventCount), baseline.samples.map(s => s.eventCount), 'Durable fact count drift');
      return { baseline, candidate };
    });
    return { scenario: scenario.id, mode: scenario.mode, metrics: metrics.flatMap(([name, value]) => {
      if (scenario.mode !== 'activated' && ['prepareMs', 'activateMs', 'shutdownMs'].includes(name)) return [];
      const baseline = summarizeSamples(pairs.map(pair => finite(value(pair.baseline))));
      const candidate = summarizeSamples(pairs.map(pair => finite(value(pair.candidate))));
      const overlap = baseline.minimumMs <= candidate.maximumMs && candidate.minimumMs <= baseline.maximumMs;
      const increasePercent = baseline.medianMs === 0 ? null : (candidate.medianMs / baseline.medianMs - 1) * 100;
      const control = name === 'firstMs' || name === 'prepareMs' || name === 'activateMs' || name === 'shutdownMs'
        || (name === 'laterMs' && scenario.mode !== 'activated');
      const investigate = name === 'rssBytes' ? candidate.medianMs - baseline.medianMs > 16 * 1024 ** 2
        : control && (increasePercent === null ? candidate.medianMs > 0 : increasePercent > 5);
      return [{ name, unit: name === 'rssBytes' ? 'bytes' : 'ms', baseline, candidate, overlap, increasePercent, investigate,
        pairedDifferences: pairs.map(pair => value(pair.candidate) - value(pair.baseline)) }];
    }) };
  });
  const target = rows.find(row => row.scenario === 'retained-100')!.metrics.find(metric => metric.name === 'laterMs')!;
  const improvementPercent = -target.increasePercent!;
  const targetStatus = target.overlap ? 'inconclusive' : improvementPercent >= 10 ? 'met' : 'not-met';
  const investigations = rows.flatMap(row => row.metrics.filter(metric => metric.investigate).map(metric => ({ scenario: row.scenario, metric: metric.name, overlap: metric.overlap })));
  return { profile: comparisonProfile, acceptance: startupAcceptance, targetStatus, improvementPercent, investigations, rows,
    performanceGate: targetStatus === 'met' && investigations.length === 0 ? 'met' : 'requires-review',
    caveats: 'Three process batches; no tail claim. RSS is maximum sampled own-process RSS per process, not tree/peak. CPU excludes hosts; concurrent intervals overlap. Structural/package/platform gates are separate. Overlapping ranges do not certify equivalence.' };
}

const root = resolve(import.meta.dir, '../..');
const sha256 = (bytes: string | Uint8Array) => createHash('sha256').update(bytes).digest('hex');
export function verifyProductionDiff(diff: string, receipt: { productionDiffSha256?: string; productionDiffNormalization?: string }): void {
  assert.ok(receipt.productionDiffNormalization === undefined || receipt.productionDiffNormalization === 'raw', 'Unknown production patch normalization');
  // Early v1 clean receipts had no patch field. Early worktree receipts
  // normalized the terminal newline; new receipts preserve exact raw bytes.
  const value = receipt.productionDiffNormalization === 'raw' ? diff : diff.trim();
  assert.equal(sha256(value), receipt.productionDiffSha256 ?? sha256(''), 'Candidate production patch drift');
}
function git(...args: string[]) {
  const result = Bun.spawnSync(['git', ...args], { cwd: root, stdout: 'pipe', stderr: 'pipe' });
  assert.equal(result.exitCode, 0, result.stderr.toString());
  return args[0] === 'diff' ? result.stdout.toString() : result.stdout.toString().trim();
}
async function headroom(directory: string) {
  const disk = await statfs(directory);
  const meminfo = process.platform === 'linux' ? await readFile('/proc/meminfo', 'utf8') : '';
  const availableBytes = Number(/^MemAvailable:\s+(\d+)\s+kB$/mu.exec(meminfo)?.[1] ?? 0) * 1024;
  assert.ok(process.platform !== 'linux' || availableBytes >= 2 * 1024 ** 3, 'Less than 2 GiB available RAM');
  assert.ok(disk.bavail * disk.bsize >= 2 * 1024 ** 3, 'Less than 2 GiB free disk');
  if (process.platform !== 'win32') {
    const processes = Bun.spawnSync(['ps', '-eo', 'comm='], { stdout: 'pipe', stderr: 'pipe' });
    assert.equal(processes.exitCode, 0);
    assert.ok(!processes.stdout.toString().split('\n').some(command => /^(cargo|rustc)$/u.test(command.trim())), 'Finish builds before measurement');
  }
  return { availableBytes, diskBytes: disk.bavail * disk.bsize, loadAverage: loadavg() };
}
async function main() {
  const args = new Map<string, string>();
  for (let i = 2; i < process.argv.length; i += 2) {
    const key = process.argv[i]!, value = process.argv[i + 1];
    assert.ok(['--baseline', '--candidate-receipt'].includes(key) && value && !args.has(key), 'Supply --baseline directory and --candidate-receipt file');
    args.set(key, value);
  }
  assert.equal(args.size, 2);
  const baselineRoot = resolve(args.get('--baseline')!);
  const baseline = JSON.parse(await readFile(join(baselineRoot, 'manifest.json'), 'utf8'));
  const complete = JSON.parse(await readFile(join(baselineRoot, 'complete.json'), 'utf8'));
  assert.equal(baseline.profile, 'woml.startup-separation-baseline/v1');
  assert.equal(baseline.phase, 'baseline'); assert.equal(complete.processes, 39); assert.equal(complete.executions, 117);
  assert.deepEqual(baseline.acceptance, startupAcceptance); assert.deepEqual(baseline.scenarios, startupScenarios());
  assert.equal(baseline.platform, process.platform); assert.equal(baseline.arch, process.arch);
  assert.equal(baseline.bunVersion, Bun.version); assert.equal(baseline.cpu, cpus()[0]?.model);
  const candidate = JSON.parse(await readFile(resolve(args.get('--candidate-receipt')!), 'utf8'));
  assert.equal(candidate.profile, 'woml.startup-separation-build/v1'); assert.equal(candidate.buildMode, 'release');
  assert.equal(candidate.sourceCommit, git('rev-parse', 'HEAD'));
  const productionDiff = git('diff', 'HEAD', '--', 'core', 'woml/src', 'woml-cli/src');
  verifyProductionDiff(productionDiff, candidate);
  if (candidate.productionPatch) assert.equal(sha256(await readFile(candidate.productionPatch, 'utf8')), candidate.productionDiffSha256);
  assert.equal(git('ls-files', '--others', '--exclude-standard', '--', 'core', 'woml/src', 'woml-cli/src'), '');
  const sourceTree = git('ls-tree', '-r', 'HEAD', '--', 'core', 'woml/src', 'woml-cli/src');
  assert.equal(candidate.sourceTreeSha256, sha256(sourceTree));
  for (const item of baseline.harness as { path: string; sha256: string }[]) {
    // Capture orchestration changes do not change the child measurement boundary.
    if (item.path.endsWith('/startup-separation-baseline.ts')) continue;
    assert.equal(sha256(await readFile(join(root, item.path))), item.sha256, `Compiler/host/measurement changed: ${item.path}`);
  }
  assert.deepEqual(baseline.fixtures.map((f: { sha256: string }) => f.sha256), startupScenarios().map(s => sha256(sessionFixture(s.case, s.concurrency).source)));
  const output = join(root, 'woml-cli/.woml/performance'); await mkdir(output, { recursive: true });
  const directory = await mkdtemp(join(output, 'startup-comparison-'));
  const nativePaths = { baseline: join(directory, 'baseline.node'), candidate: join(directory, 'candidate.node') };
  await copyFile(join(baselineRoot, 'baseline.node'), nativePaths.baseline);
  await copyFile(candidate.native.path, nativePaths.candidate);
  assert.equal(sha256(await readFile(nativePaths.baseline)), baseline.native.sha256);
  assert.equal(sha256(await readFile(nativePaths.candidate)), candidate.native.sha256);
  const manifest = { profile: comparisonProfile, createdAt: new Date().toISOString(), baseline, candidate,
    schedule: comparisonSchedule(), resources: await headroom(directory),
    provenance: 'Explicit build receipt and immutable copied artifacts, not signed independent build attestation. Same child/compiler/host fingerprints as frozen baseline. Each pair uses independent processes and fresh stores; baseline is rerun, not pooled with the historical capture.' };
  await Bun.write(join(directory, 'manifest.json'), JSON.stringify(manifest, null, 2));
  console.log(`Artifacts: ${directory}`);
  const observations: Observation[] = [];
  try {
    for (const row of manifest.schedule) {
      const resources = await headroom(directory);
      const sampleDirectory = join(directory, `${row.batch}-${row.scenario.id}-${row.variant}`); await mkdir(sampleDirectory);
      await Bun.write(join(sampleDirectory, 'resources.json'), JSON.stringify(resources));
      const env: Record<string, string | undefined> = { ...process.env, NO_COLOR: '1', WOML_PROFILE: '0' }; delete env.RUST_MIN_STACK;
      const child = Bun.spawn([process.execPath, join(import.meta.dir, 'runtime-session-profile-child.ts'),
        '--case', row.scenario.case, '--mode', row.scenario.mode, '--directory', sampleDirectory, '--native-core', nativePaths[row.variant],
        '--runs', String(startupAcceptance.runsPerProcess), '--retained', String(row.scenario.retained),
        '--concurrency', String(row.scenario.concurrency), '--idle-ms', String(row.scenario.idleMs)],
      { cwd: root, stdin: 'ignore', stdout: 'pipe', stderr: 'pipe', detached: process.platform !== 'win32', env });
      const stop = () => {
        if (process.platform === 'win32') {
          if (child.exitCode === null) Bun.spawnSync(['taskkill', '/PID', String(child.pid), '/T', '/F'], { stdout: 'ignore', stderr: 'ignore' });
        } else {
          try { process.kill(-child.pid, 'SIGKILL'); }
          catch (error) { if ((error as NodeJS.ErrnoException).code !== 'ESRCH') throw error; }
        }
      };
      const watchdog = setTimeout(stop, 300_000);
      process.once('SIGINT', stop); process.once('SIGTERM', stop);
      try {
        const [stdout, stderr] = await Promise.all([new Response(child.stdout).text(), new Response(child.stderr).text()]);
        await Bun.write(join(sampleDirectory, 'stdout.json'), stdout); await Bun.write(join(sampleDirectory, 'stderr.log'), stderr);
        assert.equal(await child.exited, 0, `${row.variant}/${row.scenario.id}: ${stderr}\n${stdout}`);
        const observation: Observation = { ...row, result: JSON.parse(stdout) }; validateObservation(observation);
        observations.push(observation);
        await Bun.write(join(directory, 'measurements.json'), JSON.stringify({ observations }, null, 2));
        console.log(`${observations.length}/78 ${row.batch} ${row.scenario.id} ${row.variant}: later=${median(observation.result.samples.slice(1).map(s => s.durationMs)).toFixed(1)} ms`);
      } finally {
        clearTimeout(watchdog); process.removeListener('SIGINT', stop); process.removeListener('SIGTERM', stop);
        stop(); if (child.exitCode === null) await child.exited;
      }
    }
    assert.equal(git('rev-parse', 'HEAD'), candidate.sourceCommit); assert.equal(git('diff', 'HEAD', '--', 'core', 'woml/src', 'woml-cli/src'), productionDiff);
    for (const item of baseline.harness as { path: string; sha256: string }[]) {
      if (!item.path.endsWith('/startup-separation-baseline.ts')) assert.equal(sha256(await readFile(join(root, item.path))), item.sha256);
    }
    assert.equal(sha256(await readFile(nativePaths.baseline)), baseline.native.sha256);
    assert.equal(sha256(await readFile(nativePaths.candidate)), candidate.native.sha256);
    const summary = compareObservations(observations);
    await Bun.write(join(directory, 'summary.json'), JSON.stringify(summary, null, 2));
    await Bun.write(join(directory, 'complete.json'), JSON.stringify({ completedAt: new Date().toISOString(), processes: observations.length, executions: observations.length * 3,
      seededExecutions: observations.reduce((sum, row) => sum + row.scenario.retained, 0), performanceGate: summary.performanceGate }));
    console.log(JSON.stringify({ directory, targetStatus: summary.targetStatus, improvementPercent: summary.improvementPercent, investigations: summary.investigations }));
    if (summary.performanceGate !== 'met') process.exitCode = 2;
  } catch (error) {
    await Bun.write(join(directory, 'failure.json'), JSON.stringify({ completedProcesses: observations.length, error: String(error) }, null, 2));
    throw error;
  }
}
if (import.meta.main) await main();
