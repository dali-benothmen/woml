#!/usr/bin/env bun
// Sequential diagnostics only. Never builds, installs, changes policy or resets a store.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { copyFile, mkdir, mkdtemp, readFile, statfs, writeFile } from 'node:fs/promises';
import { cpus, loadavg } from 'node:os';
import { join, relative, resolve } from 'node:path';
import { groupSpans, nativeBuildOrder, type Span } from './profile-runtime-investigation';
import { summarizeSamples } from './performance-measurement';
import { sessionFixture, sessionModes, type SessionMode, type SessionResult } from './runtime-session-profile-child';
import { runtimeProfileFixtures } from './runtime-profile-fixtures';

const repository = resolve(import.meta.dir, '../..');
type Build = 'baseline' | 'candidate';
export interface SessionSpan extends Span { readonly startOffsetMs: number; readonly runId?: string }

// Rust and native labels share the library's Instant origin. Host and JS clocks
// do not: never place their spans into these windows by offset comparison.
export function nativeWindowGroups(spans: readonly SessionSpan[], sample: SessionResult) {
  const native = spans.filter(span => ['rust', 'native'].includes(span.process));
  return sample.samples.map(run => {
    const name = sample.mode === 'activated' ? 'runtime.dispatch_run' : 'runtime.execute_durable';
    const windows = native.filter(span => span.name === name && span.runId === run.runId);
    assert.equal(windows.length, 1, `One native execution window for ${run.runId}`);
    const window = windows[0]!;
    const end = window.startOffsetMs + window.durationMs;
    const contained = native.filter(span => span.startOffsetMs >= window.startOffsetMs
      && span.startOffsetMs + span.durationMs <= end + 0.001);
    return { index: run.index, runId: run.runId, groups: groupSpans(contained) };
  });
}

interface Observation {
  build: Build; case: string; mode: SessionMode; batch: number; phase: string;
  directory: string; processWallMs: number; result: SessionResult;
  spanGroups?: ReturnType<typeof groupSpans>;
  nativeWindows?: ReturnType<typeof nativeWindowGroups>;
}

export function sessionSummary(observations: readonly Observation[]) {
  let text = '# Repeated-runtime investigation\n\n';
  text += 'Each observation is one independent process, one case/build/mode and several serial runs. First run is separate. Repeated-run values are the median within each process, then summarized across independent processes (not pooled as independent runs). Fresh-store still shares the local object-backend directory inside a process.\n\n';
  text += '| Case | Mode | Build | Processes | First median [min–max] ms | Repeated process-median [min–max] ms |\n| --- | --- | --- | ---: | --- | --- |\n';
  const data = observations.filter(item => item.phase === 'baseline');
  for (const id of [...new Set(data.map(item => item.case))]) for (const mode of sessionModes) for (const build of ['baseline', 'candidate']) {
    const rows = data.filter(item => item.case === id && item.mode === mode && item.build === build);
    if (!rows.length) continue;
    const first = summarizeSamples(rows.map(row => row.result.samples[0]!.durationMs));
    const warm = summarizeSamples(rows.map(row => summarizeSamples(row.result.samples.slice(1).map(run => run.durationMs)).medianMs));
    const cell = (s: typeof first) => `${s.medianMs.toFixed(2)} [${s.minimumMs.toFixed(2)}–${s.maximumMs.toFixed(2)}]`;
    text += `| ${id} | ${mode} | ${build} | ${rows.length} | ${cell(first)} | ${cell(warm)} |\n`;
  }
  text += '\nActivation preparation, activation, admission and shutdown are retained separately in measurements.json. Activated run latency is submission-to-terminal-callback; one-shot is executor-call-to-result. These are related but not identical boundaries. Native windows and span totals are inclusive and overlapping, not an exclusive wall-time breakdown.\n';
  return text;
}

function git(...args: string[]) {
  const result = Bun.spawnSync(['git', ...args], { cwd: repository, stdout: 'pipe', stderr: 'pipe' });
  assert.equal(result.exitCode, 0, result.stderr.toString());
  return result.stdout.toString().trim();
}
const digest = (input: Uint8Array | string) => createHash('sha256').update(input).digest('hex');
const paths = ['core', 'woml/src', 'woml-cli/src', 'woml-cli/scripts/runtime-profile-fixtures.ts',
  'woml-cli/scripts/profile-runtime-investigation.ts', 'woml-cli/scripts/runtime-session-profile-child.ts', 'woml-cli/scripts/profile-runtime-sessions.ts'];
async function sourceState() {
  const files = git('ls-files', '--others', '--exclude-standard', '--', ...paths).split('\n').filter(Boolean).sort();
  return { commit: git('rev-parse', 'HEAD'), trackedDiffSha256: digest(git('diff', 'HEAD', '--', ...paths)),
    untracked: await Promise.all(files.map(async path => ({ path, sha256: digest(await readFile(join(repository, path))) }))) };
}
async function resources(root: string) {
  const memory = process.platform === 'linux' ? await readFile('/proc/meminfo', 'utf8') : '';
  const availableBytes = Number(/^MemAvailable:\s+(\d+)\s+kB$/mu.exec(memory)?.[1] ?? 0) * 1024;
  const disk = await statfs(root);
  assert.ok(process.platform !== 'linux' || availableBytes >= 2 * 1024 ** 3, 'Less than 2 GiB memory available');
  assert.ok(disk.bavail * disk.bsize >= 2 * 1024 ** 3, 'Less than 2 GiB disk available');
  return { availableBytes, diskBytes: disk.bavail * disk.bsize, loadAverage: loadavg() };
}

async function main() {
  const args = new Map<string, string>(), argv = process.argv.slice(2);
  for (let i = 0; i < argv.length; i += 2) {
    const key = argv[i]!, value = argv[i + 1];
    assert.ok(['--baseline-manifest', '--candidate', '--cases', '--modes', '--runs', '--batches', '--phase'].includes(key) && value && !args.has(key), 'Invalid argument');
    args.set(key, value);
  }
  assert.ok(args.get('--baseline-manifest') && args.get('--candidate'), 'Explicit baseline manifest and candidate required');
  const cases = (args.get('--cases') ?? 'sequential-2,storage-4,sequential-32').split(',').map(sessionFixture);
  assert.equal(new Set(cases.map(item => item.id)).size, cases.length);
  const modes = (args.get('--modes') ?? sessionModes.join(',')).split(',') as SessionMode[];
  assert.ok(modes.length && modes.every(mode => sessionModes.includes(mode)));
  assert.equal(new Set(modes).size, modes.length);
  const runs = Number(args.get('--runs') ?? 5), batches = Number(args.get('--batches') ?? 3);
  assert.ok(Number.isSafeInteger(runs) && runs >= 2 && runs <= 5);
  assert.ok(Number.isSafeInteger(batches) && batches >= 1 && batches <= 3);
  const phase = args.get('--phase') ?? 'baseline';
  assert.ok(['baseline', 'trace'].includes(phase));
  const prior = JSON.parse(await readFile(resolve(args.get('--baseline-manifest')!), 'utf8'));
  assert.equal(prior.profile, 'woml.runtime-investigation/v1');
  assert.equal(prior.comparison, undefined);
  assert.match(prior.sourceCommit, /^[a-f0-9]{40}$/u);
  git('merge-base', '--is-ancestor', prior.sourceCommit, 'HEAD');
  assert.equal(git('diff', prior.sourceCommit, '--', 'woml/src', 'woml-cli/src'), '', 'Compiler and host must match baseline');
  for (const fixture of cases) assert.equal(prior.fixtures.find((item: any) => item.id === fixture.id)?.sha256,
    digest(runtimeProfileFixtures().find(original => original.id === fixture.id)!.source));
  const state = await sourceState();
  const root = resolve(repository, 'woml-cli/.woml/performance');
  await mkdir(root, { recursive: true });
  const directory = await mkdtemp(join(root, 'runtime-sessions-'));
  const before = await resources(directory);
  const native = {} as Record<Build, { path: string; source: string; sha256: string }>;
  for (const build of ['baseline', 'candidate'] as const) {
    const source = resolve(build === 'baseline' ? prior.native.release.path : args.get('--candidate')!);
    const sha256 = digest(await readFile(source));
    if (build === 'baseline') assert.equal(sha256, prior.native.release.sha256);
    else assert.notEqual(sha256, prior.native.release.sha256, 'Candidate must be explicitly rebuilt');
    const path = join(directory, `${build}.node`);
    await copyFile(source, path);
    native[build] = { source, path, sha256 };
  }
  const manifest = { profile: 'woml.runtime-sessions/v1', createdAt: new Date().toISOString(), sourceState: state,
    platform: process.platform, arch: process.arch, bunVersion: Bun.version, cpu: cpus()[0]?.model,
    phase, batches: phase === 'trace' ? 1 : batches, runs, modes, native, initialResources: before,
    baselineManifest: resolve(args.get('--baseline-manifest')!), baselineSourceCommit: prior.sourceCommit,
    fixtures: cases.map(fixture => ({ id: fixture.id, sha256: digest(fixture.source),
      originalSha256: digest(runtimeProfileFixtures().find(original => original.id === fixture.id)!.source),
      adaptation: 'Naming only: numeric hyphen segment gains n prefix for activated target registration; policy workflow allowlist narrowed to adapted workflow ID. All modes/builds use the same adapted definition.' })),
    boundaries: 'source embedding; compilation excluded; activated prepare/activate separate; run submission through terminal callback; one-shot executor call through result; own-process CPU excludes Bun hosts; no polling during runs',
    provenance: 'Explicit library inputs, source/harness fingerprints and unchanged compiler/host. No independent build attestation. No package or installed-CLI measurement.' };
  await writeFile(join(directory, 'manifest.json'), JSON.stringify(manifest, null, 2));
  await writeFile(join(directory, 'source.patch'), git('diff', prior.sourceCommit, '--', ...paths));
  for (const fixture of cases) await writeFile(join(directory, `${fixture.id}.woml`), fixture.source);
  const observations: Observation[] = [];
  const persist = async () => {
    await writeFile(join(directory, 'measurements.json'), JSON.stringify({ manifest, observations }, null, 2));
    await writeFile(join(directory, 'summary.md'), sessionSummary(observations));
  };
  await persist();
  console.log(`Artifacts: ${directory}\nSerial only; no native build, package install or runtime change.`);
  const combinations = cases.flatMap(fixture => modes.map(mode => ({ fixture, mode })));
  for (let batch = 0; batch < (phase === 'trace' ? 1 : batches); batch++) {
    const offset = batch % combinations.length;
    const ordered = [...combinations.slice(offset), ...combinations.slice(0, offset)];
    if (batch % 2) ordered.reverse();
    for (const [position, { fixture, mode }] of ordered.entries()) for (const build of nativeBuildOrder(true, batch, position) as Build[]) {
      const before = await resources(directory);
      const sampleDirectory = join(directory, `${phase}-${batch}-${position}-${build}-${fixture.id}-${mode}`);
      await mkdir(sampleDirectory);
      const tracePath = join(sampleDirectory, 'spans.ndjson');
      const started = performance.now();
      const child = Bun.spawn([process.execPath, join(import.meta.dir, 'runtime-session-profile-child.ts'),
        '--case', fixture.id, '--mode', mode, '--directory', sampleDirectory,
        '--native-core', native[build].path, '--runs', String(runs)], {
        cwd: repository, stdin: 'ignore', stdout: 'pipe', stderr: 'pipe', detached: process.platform !== 'win32',
        env: { ...process.env, NO_COLOR: '1', WOML_PROFILE: phase === 'trace' ? '1' : '0',
          WOML_PROFILE_OUTPUT: tracePath, WOML_PROFILE_PROCESS: 'cli', WOML_PROFILE_TRACE_ID: `session_${crypto.randomUUID().replaceAll('-', '')}` },
      });
      const stop = () => {
        if (process.platform === 'win32') { if (child.exitCode === null) child.kill('SIGKILL'); return; }
        try { process.kill(-child.pid, 'SIGKILL'); } catch (error) { if ((error as NodeJS.ErrnoException).code !== 'ESRCH') throw error; }
      };
      const watchdog = setTimeout(stop, 180_000);
      process.once('SIGINT', stop); process.once('SIGTERM', stop);
      try {
        const [stdout, stderr] = await Promise.all([new Response(child.stdout).text(), new Response(child.stderr).text()]);
        await writeFile(join(sampleDirectory, 'stdout.json'), stdout);
        await writeFile(join(sampleDirectory, 'stderr.log'), stderr);
        assert.equal(await child.exited, 0, `${build}/${fixture.id}/${mode}: ${stderr}\n${stdout}`);
        const result: SessionResult = JSON.parse(stdout);
        assert.equal(result.profile, 'woml.runtime-session-sample/v1');
        assert.equal(result.case, fixture.id); assert.equal(result.mode, mode); assert.equal(result.samples.length, runs);
        for (const previous of observations.filter(row => row.case === fixture.id)) assert.equal(previous.result.definitionHash, result.definitionHash);
        const spans: SessionSpan[] | undefined = phase === 'trace' ? (await readFile(tracePath, 'utf8')).trim().split(/\r?\n/u).map(line => JSON.parse(line)) : undefined;
        observations.push({ build, case: fixture.id, mode, phase, batch, directory: relative(directory, sampleDirectory), processWallMs: performance.now() - started, result,
          ...(spans ? { spanGroups: groupSpans(spans), nativeWindows: nativeWindowGroups(spans, result) } : {}) });
        await writeFile(join(sampleDirectory, 'resources.json'), JSON.stringify({ before, after: await resources(directory) }, null, 2));
        await persist();
        console.log(`${phase} ${batch} ${build} ${fixture.id}/${mode}: first=${result.samples[0]!.durationMs.toFixed(1)} ms; repeated-median=${summarizeSamples(result.samples.slice(1).map(run => run.durationMs)).medianMs.toFixed(1)} ms; startup=${((result.prepareMs ?? 0) + (result.activateMs ?? 0)).toFixed(1)} ms`);
      } finally {
        clearTimeout(watchdog); process.removeListener('SIGINT', stop); process.removeListener('SIGTERM', stop);
        stop(); if (child.exitCode === null) await child.exited;
      }
    }
  }
  assert.deepEqual(await sourceState(), state, 'Source or profiling harness changed while measuring');
  await writeFile(join(directory, 'complete.json'), JSON.stringify({ completedAt: new Date().toISOString(), observations: observations.length, executions: observations.length * runs }));
  console.log(`Complete: ${join(directory, 'summary.md')}`);
}

if (import.meta.main) await main();
