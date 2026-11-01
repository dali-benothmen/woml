#!/usr/bin/env bun
// Bounded same-source investigation. Never builds/stages native artifacts,
// modifies npm installations, changes runtime policy, or runs cases concurrently.
import assert from 'node:assert/strict';
import Ajv2020 from 'ajv/dist/2020';
import { createHash, randomUUID } from 'node:crypto';
import { copyFile, mkdir, mkdtemp, readFile, readdir, statfs, writeFile } from 'node:fs/promises';
import { cpus, loadavg, totalmem } from 'node:os';
import { join, relative, resolve } from 'node:path';
import { runtimeProfileFixtures, type RuntimeProfileFixture } from './runtime-profile-fixtures';
import { summarizeSamples } from './performance-measurement';
import { rustLibraryName } from './stage-native';

const repository = resolve(import.meta.dir, '../..');
const defaultRoot = resolve(repository, 'woml-cli/.woml/performance');
const GiB = 1024 ** 3;
type Build = 'debug' | 'release' | 'baseline' | 'candidate';
type NativeInput = { source: string; path: string; sha256: string };

export function nativeBuildOrder(comparison: boolean, batch: number, position: number): readonly Build[] {
  const builds: Build[] = comparison ? ['baseline', 'candidate'] : ['debug', 'release'];
  return (batch + position) % 2 ? builds.reverse() : builds;
}
const validateSpan = new Ajv2020({ strict: false }).compile(
  await Bun.file(join(repository, 'docs/schemas/performance-span.v1.schema.json')).json()
);
export interface InvocationSample {
  readonly temperature: string; readonly durationMs: number;
  readonly runId: string; readonly eventCount: number;
  readonly ownProcessCpuMs: { readonly user: number; readonly system: number };
}
export interface Sample {
  readonly profile: string; readonly case: string; readonly compileMs: number;
  readonly modelVersion: number; readonly definitionHash: string;
  readonly graphNodes: number; readonly governed: boolean;
  readonly samples: readonly InvocationSample[];
}
export interface Span {
  readonly profile: string; readonly process: string; readonly name: string;
  readonly durationMs: number; readonly runId?: string;
  readonly counts?: Readonly<Record<string, number>>;
}
interface Observation {
  readonly build: Build; readonly case: string;
  readonly phase: string; readonly batch: number; readonly position: number;
  readonly processWallMs: number; readonly result: Sample;
  readonly directory: string; readonly traceId?: string;
  readonly spanGroups?: ReturnType<typeof groupSpans>;
  readonly memory?: { readonly scope: string; readonly intervalMs: number; readonly peakTreeRssBytes: number };
}

export function groupSpans(spans: readonly Span[]) {
  const groups = new Map<string, { process: string; name: string; durations: number[]; counts: Record<string, number> }>();
  for (const span of spans) {
    assert.ok(validateSpan(span), `Invalid performance span: ${JSON.stringify(validateSpan.errors)}`);
    assert.ok(Number.isFinite(span.durationMs) && span.durationMs >= 0);
    const key = `${span.process}/${span.name}`;
    const group = groups.get(key) ?? { process: span.process, name: span.name, durations: [], counts: {} };
    group.durations.push(span.durationMs);
    for (const [name, value] of Object.entries(span.counts ?? {})) group.counts[name] = (group.counts[name] ?? 0) + value;
    groups.set(key, group);
  }
  // Inclusive span totals: useful within a process/name, never a wall-time pie.
  return [...groups.values()].map(group => ({ process: group.process, name: group.name,
    count: group.durations.length, inclusiveTotalMs: group.durations.reduce((sum, value) => sum + value, 0),
    summary: summarizeSamples(group.durations), counters: group.counts,
  })).sort((left, right) => right.inclusiveTotalMs - left.inclusiveTotalMs);
}

export function selectFixtures(names: string): readonly RuntimeProfileFixture[] {
  const fixtures = runtimeProfileFixtures();
  if (names === 'all') return fixtures;
  const ids = names.split(',');
  assert.equal(new Set(ids).size, ids.length, 'Duplicate cases are not permitted');
  return ids.map(id => {
    const fixture = fixtures.find(item => item.id === id);
    assert.ok(fixture, `Unknown case: ${id}`);
    return fixture;
  });
}

function git(...args: string[]) {
  const child = Bun.spawnSync(['git', ...args], { cwd: repository, stdout: 'pipe', stderr: 'pipe' });
  assert.equal(child.exitCode, 0, child.stderr.toString());
  return child.stdout.toString().trim();
}
function sha256(value: string | Uint8Array) { return createHash('sha256').update(value).digest('hex'); }

async function productionState() {
  const paths = ['core', 'woml/src', 'woml-cli/src'];
  const untracked = git('ls-files', '--others', '--exclude-standard', '--', ...paths).split('\n').filter(Boolean).sort();
  return { trackedDiffSha256: sha256(git('diff', 'HEAD', '--', ...paths)),
    untracked: await Promise.all(untracked.map(async path => ({ path, sha256: sha256(await readFile(join(repository, path))) }))),
  };
}

async function resources(directory: string) {
  const memory = process.platform === 'linux' ? await readFile('/proc/meminfo', 'utf8') : '';
  const availableBytes = Number(/^MemAvailable:\s+(\d+)\s+kB$/mu.exec(memory)?.[1] ?? 0) * 1024;
  const disk = await statfs(directory);
  return { availableBytes, freeDiskBytes: disk.bavail * disk.bsize, loadAverage: loadavg() };
}
async function checkResources(directory: string) {
  const current = await resources(directory);
  if (process.platform === 'linux') assert.ok(current.availableBytes >= 2 * GiB, 'Less than 2 GiB RAM available; stop and retry after freeing memory');
  assert.ok(current.freeDiskBytes >= 2 * GiB, 'Less than 2 GiB disk free; stop rather than filling the drive');
  return current;
}

async function treeRss(pid: number): Promise<number> {
  // Children may be created by a Tokio thread, not the main thread. Inspect
  // every task's children, or the Bun script host can be missed entirely.
  const pending = [pid], visited = new Set<number>();
  let rss = 0;
  while (pending.length) {
    const current = pending.pop()!;
    if (visited.has(current)) continue;
    visited.add(current);
    try {
      const status = await readFile(`/proc/${current}/status`, 'utf8');
      rss += Number(/^VmRSS:\s+(\d+)\s+kB$/mu.exec(status)?.[1] ?? 0) * 1024;
      for (const task of await readdir(`/proc/${current}/task`)) {
        try {
          const children = await readFile(`/proc/${current}/task/${task}/children`, 'utf8');
          pending.push(...children.trim().split(/\s+/u).map(Number).filter(id => Number.isSafeInteger(id) && id > 0));
        } catch { /* A task can exit while its children are sampled. */ }
      }
    } catch { /* A process can exit during sampling. */ }
  }
  return rss;
}

function reportMarkdown(observations: readonly Observation[]): string {
  let text = '# Runtime investigation measurements\n\n';
  text += 'All engine samples use fresh stores and fresh one-shot script hosts. Cold and process-warmed invocations share one benchmark process/native addon; separate batches start independent processes. Compile time is excluded from engine time. Tracing and memory sampling run separately.\n\n';
  text += '| Case | Build | Invocation | Samples | Median ms | Min–max ms | Events |\n| --- | --- | --- | ---: | ---: | --- | --- |\n';
  const baseline = observations.filter(item => item.phase === 'baseline');
  for (const id of [...new Set(baseline.map(item => item.case))]) {
    for (const build of [...new Set(observations.map(item => item.build))]) {
      for (const index of [0, 1]) {
        const samples = baseline.filter(item => item.case === id && item.build === build).map(item => item.result.samples[index]!);
        if (!samples.length) continue;
        const stats = summarizeSamples(samples.map(item => item.durationMs));
        text += `| ${id} | ${build} | ${index === 0 ? 'cold process' : 'warm process'} | ${stats.count} | ${stats.medianMs.toFixed(2)} | ${stats.minimumMs.toFixed(2)}–${stats.maximumMs.toFixed(2)} | ${[...new Set(samples.map(item => item.eventCount))].join(', ')} |\n`;
      }
    }
  }
  text += '\n## Diagnostic traces\n\nTotals below are inclusive, overlap, and include both invocations. Do not add rows or compare monotonic offsets across processes. A traced run is diagnostic, not another baseline sample.\n';
  for (const item of observations.filter(item => item.phase === 'trace')) {
    text += `\n### ${item.case} / ${item.build}\n\n| Process | Span | Calls | Inclusive total ms |\n| --- | --- | ---: | ---: |\n`;
    for (const group of item.spanGroups?.slice(0, 12) ?? []) text += `| ${group.process} | ${group.name} | ${group.count} | ${group.inclusiveTotalMs.toFixed(2)} |\n`;
  }
  text += '\n## Memory diagnostics\n\nRSS is a sampled sum over Linux descendant processes, includes shared pages more than once, and is not PSS or a guaranteed peak. Memory diagnostics are not baseline latency samples.\n';
  for (const item of observations.filter(item => item.phase === 'memory')) text += `\n- ${item.case} / ${item.build}: ${((item.memory?.peakTreeRssBytes ?? 0) / 1024 ** 2).toFixed(2)} MiB sampled tree RSS.\n`;
  return text;
}

async function main() {
  const arguments_ = process.argv.slice(2);
  const allowed = new Set(['--cases', '--batches', '--phase', '--debug', '--release', '--output', '--baseline-manifest', '--candidate']);
  const options = new Map<string, string>();
  for (let index = 0; index < arguments_.length; index += 2) {
    const name = arguments_[index]!, value = arguments_[index + 1];
    assert.ok(allowed.has(name) && value && !value.startsWith('--') && !options.has(name), `Invalid or repeated option: ${name}`);
    options.set(name, value);
  }
  const fixtures = selectFixtures(options.get('--cases') ?? 'sequential-2,sequential-8,combined');
  const batches = Number(options.get('--batches') ?? 3);
  assert.ok(Number.isSafeInteger(batches) && batches >= 1 && batches <= 5, '--batches must be 1–5; fewer than three is diagnostic only');
  const phase = options.get('--phase') ?? 'baseline';
  assert.ok(['baseline', 'trace', 'memory', 'all'].includes(phase), 'Unknown phase');
  const sourceCommit = git('rev-parse', 'HEAD');
  const dirtyProduction = git('diff', 'HEAD', '--', 'core', 'woml/src', 'woml-cli/src');
  const comparison = options.has('--baseline-manifest');
  assert.equal(options.has('--candidate'), comparison, 'Comparison requires both --baseline-manifest and --candidate');
  if (!comparison) assert.equal(dirtyProduction, '', 'Production source has changed; rebuild and establish a new source identity first');
  else assert.equal(git('diff', 'HEAD', '--', 'woml/src', 'woml-cli/src'), '', 'History comparison must keep the compiler and host unchanged');
  const sourceState = await productionState();
  const prior = comparison ? JSON.parse(await readFile(resolve(options.get('--baseline-manifest')!), 'utf8')) : undefined;
  if (prior) {
    assert.equal(prior.profile, 'woml.runtime-investigation/v1');
    assert.match(prior.sourceCommit, /^[a-f0-9]{40}$/u);
    git('merge-base', '--is-ancestor', prior.sourceCommit, sourceCommit);
    assert.equal(git('diff', prior.sourceCommit, '--', 'woml/src', 'woml-cli/src'), '', 'Compiler and host must match the retained baseline commit');
    assert.equal(prior.comparison, undefined, 'Use the original clean-source baseline manifest');
    for (const fixture of fixtures) assert.equal(prior.fixtures.find((item: { id: string }) => item.id === fixture.id)?.sha256, sha256(fixture.source), 'Fixture differs from baseline');
  }
  await mkdir(defaultRoot, { recursive: true });
  const directory = options.has('--output') ? resolve(options.get('--output')!) : await mkdtemp(join(defaultRoot, 'runtime-investigation-'));
  if (options.has('--output')) await mkdir(directory, { recursive: false });
  await checkResources(directory);
  const native: Partial<Record<Build, NativeInput>> = {};
  const builds = nativeBuildOrder(comparison, 0, 0);
  for (const build of builds) {
    const source = resolve(build === 'baseline' ? prior.native.release.path
      : options.get(`--${build}`) ?? join(repository, 'dist/target', build, rustLibraryName(process.platform)));
    const digest = sha256(await readFile(source));
    if (build === 'baseline') assert.equal(digest, prior.native.release.sha256, 'Retained baseline release binary changed');
    if (build === 'candidate') assert.notEqual(digest, prior.native.release.sha256, 'Candidate is identical to baseline; explicitly build the changed source first');
    const target = join(directory, `${build}.node`);
    await copyFile(source, target);
    native[build] = { source, path: target, sha256: digest };
  }
  const fixtureDirectory = join(directory, 'fixtures');
  await mkdir(fixtureDirectory);
  for (const fixture of fixtures) await writeFile(join(fixtureDirectory, `${fixture.id}.woml`), fixture.source);
  const sourceFiles = ['runtime-profile-fixtures.ts', 'runtime-profile-child.ts', 'profile-runtime-investigation.ts'];
  const manifest = { profile: 'woml.runtime-investigation/v1', sourceCommit,
    createdAt: new Date().toISOString(), platform: process.platform, architecture: process.arch,
    bunVersion: Bun.version, cpu: cpus()[0]?.model, logicalCpuCount: cpus().length, totalMemoryBytes: totalmem(),
    hostArtifact: 'source', scriptTimeoutMs: 30_000, batches, phase,
    ...(comparison ? { comparison: 'baseline-release-versus-candidate-release', baselineSourceCommit: prior.sourceCommit, baselineManifest: resolve(options.get('--baseline-manifest')!) } : {}),
    sourceState,
    execution: 'native-one-shot; fresh-host-and-store-per-invocation; second-invocation-warms-process-only',
    buildProvenance: comparison
      ? 'Retained original release versus an explicitly built candidate from sourceCommit plus sourceState. Hashes identify inputs, not build attestation.'
      : 'Native hashes identify inputs; rebuild both from sourceCommit before use. The runner does not attest how existing binaries were built.',
    sampling: 'Three independent cold processes per build/case by default, each followed by one process-warmed fresh-store execution. p95 from three values is only the observed maximum.',
    native, fixtures: fixtures.map(item => ({ id: item.id, sha256: sha256(item.source), governed: item.governed })),
    harness: await Promise.all(sourceFiles.map(async file => ({ path: file, sha256: sha256(await readFile(join(import.meta.dir, file))) }))),
    initialResources: await resources(directory),
  };
  await writeFile(join(directory, 'manifest.json'), JSON.stringify(manifest, null, 2));
  if (comparison) await writeFile(join(directory, 'candidate-source.patch'), git('diff', prior.sourceCommit, '--', 'core', 'woml/src', 'woml-cli/src'));
  const observations: Observation[] = [];
  const persist = async () => {
    await writeFile(join(directory, 'measurements.json'), JSON.stringify({ manifest, observations }, null, 2));
    await writeFile(join(directory, 'summary.md'), reportMarkdown(observations));
  };
  await persist();
  console.log(`Artifacts: ${directory}\nCommit: ${sourceCommit}\nNo native build or global installation is performed by this command.`);

  async function run(fixture: RuntimeProfileFixture, build: Build, phase: string, batch: number, position: number) {
    const before = await checkResources(directory);
    const sampleDirectory = join(directory, `${phase}-${batch}-${position}-${build}-${fixture.id}`);
    await mkdir(sampleDirectory);
    const traceId = `runtime_${randomUUID().replaceAll('-', '')}`;
    const trace = join(sampleDirectory, 'spans.ndjson');
    const env = { ...process.env, NO_COLOR: '1', WOML_PROFILE: phase === 'trace' ? '1' : '0',
      WOML_PROFILE_OUTPUT: trace, WOML_PROFILE_PROCESS: 'cli', WOML_PROFILE_TRACE_ID: traceId };
    const started = performance.now();
    const child = Bun.spawn([process.execPath, join(import.meta.dir, 'runtime-profile-child.ts'),
      '--case', fixture.id, '--directory', sampleDirectory, '--native-core', native[build]!.path,
    ], { cwd: repository, env, stdout: 'pipe', stderr: 'pipe', stdin: 'ignore', detached: process.platform !== 'win32' });
    const stopOwnedGroup = () => {
      if (process.platform === 'win32') { if (child.exitCode === null) child.kill('SIGKILL'); return; }
      try { process.kill(-child.pid, 'SIGKILL'); }
      catch (error) { if ((error as NodeJS.ErrnoException).code !== 'ESRCH') throw error; }
    };
    const watchdog = setTimeout(stopOwnedGroup, 180_000);
    process.once('SIGINT', stopOwnedGroup);
    process.once('SIGTERM', stopOwnedGroup);
    let peak = 0, sampling = true;
    const memoryTask = phase === 'memory' && process.platform === 'linux' ? (async () => {
      while (sampling) { peak = Math.max(peak, await treeRss(child.pid)); await Bun.sleep(100); }
    })() : undefined;
    let stdout = '', stderr = '';
    try {
      [stdout, stderr] = await Promise.all([new Response(child.stdout).text(), new Response(child.stderr).text()]);
      assert.equal(await child.exited, 0, `${phase} ${build} ${fixture.id} failed:\n${stderr}\n${stdout}`);
      const processWallMs = performance.now() - started;
      const lines = stdout.trim().split(/\r?\n/u);
      assert.equal(lines.length, 1, 'Benchmark result must be one JSON record, without workflow logging');
      const result = JSON.parse(lines[0]!) as Sample;
      assert.equal(result.profile, 'woml.runtime-profile-sample/v1');
      assert.equal(result.case, fixture.id);
      assert.equal(result.samples.length, 2);
      for (const other of observations.filter(item => item.case === fixture.id)) {
        assert.equal(result.definitionHash, other.result.definitionHash, 'Compiled fixture identity changed between samples');
        assert.equal(result.modelVersion, other.result.modelVersion);
      }
      const spanGroups = phase === 'trace' ? groupSpans((await readFile(trace, 'utf8')).trim().split(/\r?\n/u).map(line => JSON.parse(line))) : undefined;
      observations.push({ build, case: fixture.id, phase, batch, position, processWallMs, result,
        directory: relative(directory, sampleDirectory),
        ...(phase === 'trace' ? { traceId, spanGroups } : {}),
        ...(phase === 'memory' ? { memory: { scope: process.platform === 'linux' ? 'sampled-linux-process-tree-rss' : 'unavailable', intervalMs: 100, peakTreeRssBytes: peak } } : {}),
      });
      await writeFile(join(sampleDirectory, 'resources.json'), JSON.stringify({ before, after: await resources(directory) }, null, 2));
      await persist();
      console.log(`${phase} batch=${batch} ${build} ${fixture.id}: cold=${result.samples[0]!.durationMs.toFixed(1)} ms; warm-process=${result.samples[1]!.durationMs.toFixed(1)} ms; process=${processWallMs.toFixed(1)} ms`);
    } finally {
      clearTimeout(watchdog);
      process.removeListener('SIGINT', stopOwnedGroup);
      process.removeListener('SIGTERM', stopOwnedGroup);
      sampling = false;
      await memoryTask;
      stopOwnedGroup();
      if (child.exitCode === null) await child.exited;
      await writeFile(join(sampleDirectory, 'stdout.json'), stdout);
      await writeFile(join(sampleDirectory, 'stderr.log'), stderr);
    }
  }
  if (phase === 'baseline' || phase === 'all') {
    for (let batch = 0; batch < batches; batch += 1) {
      const rotated = [...fixtures.slice(batch % fixtures.length), ...fixtures.slice(0, batch % fixtures.length)];
      if (batch % 2) rotated.reverse();
      for (let position = 0; position < rotated.length; position += 1) {
        const order = nativeBuildOrder(comparison, batch, position);
        for (const build of order) await run(rotated[position]!, build, 'baseline', batch, position);
      }
    }
  }
  for (const diagnostic of ['trace', 'memory']) {
    if (phase !== diagnostic && phase !== 'all') continue;
    for (let position = 0; position < fixtures.length; position += 1) {
      for (const build of builds) await run(fixtures[position]!, build, diagnostic, 0, position);
    }
  }
  assert.equal(git('rev-parse', 'HEAD'), sourceCommit, 'Source commit changed while measuring');
  assert.deepEqual(await productionState(), sourceState, 'Production source (including new helpers) changed while measuring');
  assert.deepEqual(await Promise.all(sourceFiles.map(async file => ({ path: file, sha256: sha256(await readFile(join(import.meta.dir, file))) }))), manifest.harness, 'Harness changed while measuring');
  await writeFile(join(directory, 'complete.json'), JSON.stringify({ completedAt: new Date().toISOString(), observations: observations.length }));
  console.log(`Complete: ${join(directory, 'summary.md')}`);
}

if (import.meta.main) await main();
