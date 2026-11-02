#!/usr/bin/env bun
// SRS0: explicit native input, no builds, no package installs, no production edits.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { copyFile, mkdir, mkdtemp, readFile, stat, statfs, writeFile } from 'node:fs/promises';
import { cpus, loadavg } from 'node:os';
import { join, resolve } from 'node:path';
import { sessionFixture, sessionModes, type SessionMode, type SessionResult } from './runtime-session-profile-child';
import { summarizeSamples } from './performance-measurement';
import { groupSpans, type Span } from './profile-runtime-investigation';

const root = resolve(import.meta.dir, '../..');
const sha256 = (value: string | Uint8Array) => createHash('sha256').update(value).digest('hex');
export const startupAcceptance = {
  profile: 'woml.startup-separation-acceptance/v1',
  batches: 3, runsPerProcess: 3,
  structural: 'No full initialization, index rebuilding, or global recovery in converted steady-state request/heartbeat paths.',
  target: 'At least 10% lower median-of-process-medians for activated sequential-2 with 100 retained runs.',
  controls: 'Investigate any >5% median regression in fresh/reused one-shot, activation, first request or shutdown; overlapping ranges are inconclusive, not a pass.',
  memory: 'Investigate >16 MiB growth in sampled own-process RSS; this excludes hosts/workers and is not a peak/tree or leak certificate.',
  evidence: 'All samples retained. Baseline/candidate matched by scenario, independent process batch and toolchain. No tail-percentile claims from two later requests.',
  budgets: 'Existing release and package budgets remain unchanged.',
} as const;

export interface StartupScenario { id: string; case: string; mode: SessionMode; retained: number; concurrency: number; idleMs: number }
export function startupScenarios(): StartupScenario[] {
  const basic = ['sequential-2', 'storage-4', 'sequential-32'].flatMap(id =>
    sessionModes.map(mode => ({ id: `${id}-${mode}`, case: id, mode, retained: 0, concurrency: 1, idleMs: 0 })));
  return [...basic,
    ...[20, 100].map(retained => ({ id: `retained-${retained}`, case: 'sequential-2', mode: 'activated' as const, retained, concurrency: 1, idleMs: 0 })),
    { id: 'concurrent-2', case: 'sequential-2', mode: 'activated', retained: 0, concurrency: 2, idleMs: 0 },
    { id: 'idle-heartbeat', case: 'sequential-2', mode: 'activated', retained: 0, concurrency: 1, idleMs: 4500 },
  ];
}

export function startupSummary(observations: readonly { scenario: StartupScenario; batch: number; result: SessionResult }[]) {
  return startupScenarios().flatMap(scenario => {
    const rows = observations.filter(row => row.scenario.id === scenario.id);
    if (!rows.length) return [];
    const summary = (values: number[]) => summarizeSamples(values);
    return [{ scenario: scenario.id, processes: rows.length,
      first: summary(rows.map(row => row.result.samples[0]!.durationMs)),
      laterProcessMedians: summary(rows.map(row => summary(row.result.samples.slice(1).map(sample => sample.durationMs)).medianMs)),
      prepare: rows.every(row => row.result.prepareMs !== undefined) ? summary(rows.map(row => row.result.prepareMs!)) : undefined,
      shutdown: rows.every(row => row.result.shutdownMs !== undefined) ? summary(rows.map(row => row.result.shutdownMs!)) : undefined,
    }];
  });
}

function git(...args: string[]) {
  const result = Bun.spawnSync(['git', ...args], { cwd: root, stdout: 'pipe', stderr: 'pipe' });
  assert.equal(result.exitCode, 0, result.stderr.toString());
  return result.stdout.toString().trim();
}
async function resources(directory: string) {
  const disk = await statfs(directory);
  const memory = process.platform === 'linux' ? await readFile('/proc/meminfo', 'utf8') : '';
  const availableBytes = Number(/^MemAvailable:\s+(\d+)\s+kB$/mu.exec(memory)?.[1] ?? 0) * 1024;
  assert.ok(process.platform !== 'linux' || availableBytes >= 2 * 1024 ** 3, 'Less than 2 GiB available RAM');
  assert.ok(disk.bavail * disk.bsize >= 2 * 1024 ** 3, 'Less than 2 GiB free disk');
  return { availableBytes, diskBytes: disk.bavail * disk.bsize, loadAverage: loadavg() };
}

async function main() {
  const args = new Map<string, string>();
  for (let index = 2; index < process.argv.length; index += 2) {
    const key = process.argv[index]!, value = process.argv[index + 1];
    assert.ok(['--native', '--phase', '--scenarios'].includes(key) && value && !args.has(key), 'Invalid argument');
    args.set(key, value);
  }
  assert.ok(args.get('--native'), 'Supply an explicitly rebuilt optimized --native library');
  const phase = args.get('--phase') ?? 'baseline';
  assert.ok(['baseline', 'trace'].includes(phase), 'Unknown phase');
  const scenarios = args.has('--scenarios') ? args.get('--scenarios')!.split(',').map(id => {
    const scenario = startupScenarios().find(item => item.id === id);
    assert.ok(scenario, `Unknown scenario ${id}`); return scenario;
  }) : startupScenarios();
  assert.equal(new Set(scenarios.map(item => item.id)).size, scenarios.length);
  const productionPaths = ['core', 'woml/src', 'woml-cli/src'];
  assert.equal(git('diff', 'HEAD', '--', ...productionPaths), '', 'Establish the baseline before production edits');
  assert.equal(git('ls-files', '--others', '--exclude-standard', '--', ...productionPaths), '', 'Untracked production source needs a reviewed baseline');
  const nativeSource = resolve(args.get('--native')!);
  const nativeHash = sha256(await readFile(nativeSource));
  const harnessPaths = ['scripts/startup-separation-baseline.ts', 'scripts/runtime-session-profile-child.ts', 'scripts/runtime-profile-fixtures.ts',
    'scripts/performance-measurement.ts', 'scripts/profile-runtime-investigation.ts', 'src'];
  const compilerHostDiff = git('diff', 'HEAD', '--', 'woml/src', 'woml-cli/src');
  const harnessFiles = git('ls-files', '--', 'woml/src', 'woml-cli/src').split('\n').filter(Boolean)
    .concat(harnessPaths.filter(path => path !== 'src').map(path => `woml-cli/${path}`));
  const fingerprints = async () => Promise.all(harnessFiles.map(async path => ({ path, sha256: sha256(await readFile(join(root, path))) })));
  const harness = await fingerprints();
  const outputRoot = join(root, 'woml-cli/.woml/performance');
  await mkdir(outputRoot, { recursive: true });
  const directory = await mkdtemp(join(outputRoot, `startup-${phase}-`));
  const nativePath = join(directory, 'baseline.node');
  await copyFile(nativeSource, nativePath);
  const manifest = { profile: 'woml.startup-separation-baseline/v1', phase, createdAt: new Date().toISOString(),
    sourceCommit: git('rev-parse', 'HEAD'), productionDiffSha256: sha256(''),
    platform: process.platform, arch: process.arch, cpu: cpus()[0]?.model, bunVersion: Bun.version,
    native: { source: nativeSource, sha256: nativeHash, mtime: (await stat(nativeSource)).mtime.toISOString() },
    provenance: 'Explicit Cargo release build performed separately; capture does not independently attest compilation. Immutable copied library, clean production source at capture, fingerprinted compiler/host/harness.',
    acceptance: startupAcceptance, batches: phase === 'baseline' ? startupAcceptance.batches : 1, scenarios,
    fixtures: scenarios.map(scenario => ({ scenario: scenario.id, source: sessionFixture(scenario.case, scenario.concurrency).source,
      sha256: sha256(sessionFixture(scenario.case, scenario.concurrency).source), adaptation: 'Activation-compatible ID; concurrent probe alone declares workflow concurrency=2. Retained probes seed real finalized one-step histories before activation.' })),
    harness, resources: await resources(directory),
    boundaries: 'Compilation and retained-history seeding excluded; prepare, activate, first and later requests, idle and shutdown separate. Activated submission-to-terminal callback differs from one-shot executor-to-result. Concurrent CPU samples overlap and must not be summed. Own-process RSS excludes host tree.',
  };
  await writeFile(join(directory, 'manifest.json'), JSON.stringify(manifest, null, 2));
  const observations: { scenario: StartupScenario; batch: number; result: SessionResult; spans?: ReturnType<typeof groupSpans> }[] = [];
  const persist = async () => {
    await writeFile(join(directory, 'measurements.json'), JSON.stringify({ observations }, null, 2));
    await writeFile(join(directory, 'summary.json'), JSON.stringify(startupSummary(observations), null, 2));
  };
  console.log(`Artifacts: ${directory}`);
  for (let batch = 0; batch < manifest.batches; batch++) {
    const ordered = batch % 2 ? [...scenarios].reverse() : scenarios;
    for (const scenario of ordered) {
      await resources(directory);
      const sampleDirectory = join(directory, `${batch}-${scenario.id}`);
      await mkdir(sampleDirectory);
      const trace = join(sampleDirectory, 'spans.ndjson');
      const child = Bun.spawn([process.execPath, join(import.meta.dir, 'runtime-session-profile-child.ts'),
        '--case', scenario.case, '--mode', scenario.mode, '--directory', sampleDirectory, '--native-core', nativePath,
        '--runs', String(startupAcceptance.runsPerProcess), '--retained', String(scenario.retained),
        '--concurrency', String(scenario.concurrency), '--idle-ms', String(scenario.idleMs)], {
        cwd: root, stdin: 'ignore', stdout: 'pipe', stderr: 'pipe', detached: process.platform !== 'win32',
        env: { ...process.env, NO_COLOR: '1', WOML_PROFILE: phase === 'trace' ? '1' : '0',
          WOML_PROFILE_OUTPUT: trace, WOML_PROFILE_PROCESS: 'cli', WOML_PROFILE_TRACE_ID: `startup_${crypto.randomUUID().replaceAll('-', '')}` },
      });
      const stop = () => {
        if (process.platform === 'win32') {
          if (child.exitCode === null) Bun.spawnSync(['taskkill', '/pid', String(child.pid), '/T', '/F'], { stdout: 'ignore', stderr: 'ignore' });
        } else {
          try { process.kill(-child.pid, 'SIGKILL'); }
          catch (error) { if ((error as NodeJS.ErrnoException).code !== 'ESRCH') throw error; }
        }
      };
      const watchdog = setTimeout(stop, 300_000);
      process.once('SIGINT', stop); process.once('SIGTERM', stop);
      try {
        const [stdout, stderr] = await Promise.all([new Response(child.stdout).text(), new Response(child.stderr).text()]);
        await writeFile(join(sampleDirectory, 'stdout.json'), stdout);
        await writeFile(join(sampleDirectory, 'stderr.log'), stderr);
        assert.equal(await child.exited, 0, `${scenario.id}: ${stderr}\n${stdout}`);
        const result: SessionResult = JSON.parse(stdout);
        assert.equal(result.profile, 'woml.runtime-session-sample/v1');
        assert.equal(result.case, scenario.case); assert.equal(result.mode, scenario.mode);
        assert.equal(result.samples.length, startupAcceptance.runsPerProcess);
        assert.deepEqual(result.parameters, { retained: scenario.retained, concurrency: scenario.concurrency, idleMs: scenario.idleMs });
        assert.ok(result.samples.every(sample => Number.isFinite(sample.durationMs) && sample.durationMs > 0 && sample.eventCount! > 0));
        const spans = phase === 'trace' ? groupSpans((await readFile(trace, 'utf8')).trim().split('\n').map(line => JSON.parse(line) as Span)) : undefined;
        observations.push({ scenario, batch, result, ...(spans ? { spans } : {}) });
        await persist();
        console.log(`${phase} ${batch} ${scenario.id}: first=${result.samples[0]!.durationMs.toFixed(1)} ms; later=${summarizeSamples(result.samples.slice(1).map(sample => sample.durationMs)).medianMs.toFixed(1)} ms`);
      } finally {
        clearTimeout(watchdog); process.removeListener('SIGINT', stop); process.removeListener('SIGTERM', stop);
        stop(); if (child.exitCode === null) await child.exited;
      }
    }
  }
  assert.deepEqual(await fingerprints(), harness, 'Loaded compiler/host/harness changed while measuring');
  assert.equal(git('diff', 'HEAD', '--', 'woml/src', 'woml-cli/src'), compilerHostDiff);
  assert.equal(sha256(await readFile(nativePath)), nativeHash);
  await writeFile(join(directory, 'complete.json'), JSON.stringify({ completedAt: new Date().toISOString(), processes: observations.length,
    executions: observations.length * startupAcceptance.runsPerProcess, seededExecutions: observations.reduce((sum, row) => sum + row.scenario.retained, 0) }));
  console.log(`Complete: ${join(directory, 'summary.json')}`);
}
if (import.meta.main) await main();
