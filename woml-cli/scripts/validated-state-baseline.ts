#!/usr/bin/env bun
// Build is explicit; capture never compiles, installs, or selects a stale addon.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { copyFile, mkdir, mkdtemp, readFile, statfs } from 'node:fs/promises';
import { freemem, platform, arch, cpus, hostname } from 'node:os';
import { join, resolve } from 'node:path';
import acceptance from '../../docs/validated-state-reuse-acceptance.v1.json';

const root = resolve(import.meta.dir, '../..');
const sha256 = (value: string | Uint8Array) => createHash('sha256').update(value).digest('hex');
const testName = 'durable::boundary_tests::validated_state_baseline::capture';
const harnessFiles = ['.cargo/config.toml', 'woml-cli/scripts/validated-state-baseline.ts', 'docs/validated-state-reuse-acceptance.v1.json'];
const sourceRoots = ['core', 'woml/src', 'woml/tests/fixtures', 'woml-cli/src'];

function command(argv: string[]) {
  const result = Bun.spawnSync(argv, { cwd: root, stdout: 'pipe', stderr: 'pipe' });
  assert.equal(result.exitCode, 0, result.stderr.toString());
  return result.stdout.toString();
}

export async function snapshot() {
  const paths = [...new Set([...command(['git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z', '--', ...sourceRoots]).split('\0').filter(Boolean), ...harnessFiles])].sort();
  const files = await Promise.all(paths.map(async path => ({ path, sha256: sha256(await readFile(join(root, path))) })));
  return { commit: command(['git', 'rev-parse', 'HEAD']).trim(), files, sha256: sha256(JSON.stringify(files)) };
}

export async function headroom() {
  let memory = freemem();
  if (process.platform === 'linux') {
    const info = await readFile('/proc/meminfo', 'utf8');
    const match = info.match(/^MemAvailable:\s+(\d+) kB$/m);
    assert.ok(match); memory = Number(match[1]) * 1024;
  }
  const disk = await statfs(root);
  const availableDisk = disk.bavail * disk.bsize;
  assert.ok(memory >= acceptance.minimumAvailableMemoryBytes, `Need at least 2 GiB available RAM; found ${(memory / 2 ** 30).toFixed(2)} GiB`);
  assert.ok(availableDisk >= acceptance.minimumAvailableDiskBytes, 'Need at least 2 GiB available disk');
  return { availableMemoryBytes: memory, availableDiskBytes: availableDisk };
}

export function validateCapture(value: unknown): asserts value is { rows: Record<string, any>[] } {
  assert.ok(value && typeof value === 'object');
  const report = value as Record<string, any>;
  assert.equal(report.profile, 'woml.validated-state-baseline/v1');
  assert.equal(report.allocationMeasurement, null, 'No allocator totals are measured by this harness');
  assert.equal(report.peakLiveBytes, null);
  assert.ok(Array.isArray(report.rows));
  assert.equal(report.rows.length, acceptance.eventLengths.length * (acceptance.warmupConnections + acceptance.measuredConnections));
  const timing = (sample: any) => {
    assert.ok(sample && Number.isFinite(sample.wallMs) && sample.wallMs >= 0);
    assert.ok(sample.threadCpuMs === null || (Number.isFinite(sample.threadCpuMs) && sample.threadCpuMs >= 0));
  };
  const identities = new Map<number, string>();
  for (const [position, row] of report.rows.entries()) {
    const count = acceptance.warmupConnections + acceptance.measuredConnections;
    assert.equal(row.events, acceptance.eventLengths[Math.floor(position / count)]);
    assert.equal(row.sample, position % count);
    assert.equal(row.warmup, row.sample < acceptance.warmupConnections);
    assert.match(row.fixtureSha256, /^[a-f0-9]{64}$/);
    const prior = identities.get(row.events);
    if (prior) assert.equal(prior, row.fixtureSha256, 'Fixture identity changed within capture');
    identities.set(row.events, row.fixtureSha256);
    timing(row.miss); timing(row.structuralFold);
    assert.ok(Array.isArray(row.hits)); assert.equal(row.hits.length, acceptance.hitsPerConnection);
    row.hits.forEach(timing);
  }
}

export function resourceRecord(usage: Pick<Bun.ResourceUsage, 'maxRSS' | 'cpuTime' | 'contextSwitches'> | undefined) {
  if (!usage) return null;
  // Bun exposes non-enumerable getters: JSON.stringify(usage) can produce {}.
  // Materialize fields and units explicitly; unavailable data is never zero.
  const finite = (value: unknown) => typeof value === 'bigint' ? (value >= 0n ? value.toString() : null)
    : typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : null;
  // Bun 1.3.14 on this Linux host returns raw rusage-like RSS and BigInt CPU
  // fields despite the installed typings. Do not assert a byte unit from those
  // typings or silently lose BigInts. RSS cannot enter a byte budget yet.
  return { maxRssRaw: finite(usage.maxRSS), maxRssUnit: 'unverified-runtime-unit', bun: Bun.version, cpuTimeMicroseconds: {
    user: finite(usage.cpuTime.user), system: finite(usage.cpuTime.system), total: finite(usage.cpuTime.total),
  }, contextSwitches: { voluntary: finite(usage.contextSwitches.voluntary), involuntary: finite(usage.contextSwitches.involuntary) } };
}

function buildEnvironment() {
  for (const name of ['CARGO_TARGET_DIR', 'CARGO_BUILD_TARGET', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'RUSTC', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER']) assert.ok(!process.env[name], `Unset ${name}`);
  assert.ok(!Object.keys(process.env).some(name => name.startsWith('CARGO_PROFILE_RELEASE_')), 'Custom release profile not supported');
}

async function build() {
  buildEnvironment(); await headroom();
  const before = await snapshot();
  const toolchain = { rustc: command(['rustc', '--version']).trim(), cargo: command(['cargo', '--version']).trim(), bun: Bun.version };
  const parent = join(root, 'woml-cli/.woml/performance'); await mkdir(parent, { recursive: true });
  const directory = await mkdtemp(join(parent, 'validated-state-build-'));
  const argv = ['cargo', 'test', '--offline', '-j', '1', '--locked', '--manifest-path', 'core/Cargo.toml', '-p', 'woml-engine', '--release', '--lib', '--no-run', '--message-format=json'];
  const child = Bun.spawn(argv, { cwd: root, stdout: 'pipe', stderr: 'inherit' });
  const stdout = await new Response(child.stdout).text();
  await Bun.write(join(directory, 'cargo.jsonl'), stdout);
  assert.equal(await child.exited, 0, `Build failed; retained ${directory}`);
  assert.deepEqual(await snapshot(), before, 'Source changed during build');
  const artifacts = stdout.split('\n').filter(Boolean).map(line => JSON.parse(line))
    .filter(item => item.reason === 'compiler-artifact' && item.target?.name === 'woml_engine' && item.profile?.test && item.executable);
  assert.equal(artifacts.length, 1, 'Expected one exact engine unit-test executable');
  const binary = join(directory, process.platform === 'win32' ? 'baseline.exe' : 'baseline');
  await copyFile(artifacts[0].executable, binary);
  const trackedPatch = command(['git', 'diff', 'HEAD', '--', ...sourceRoots]);
  const receipt = { profile: 'woml.validated-state-build/v1', buildMode: 'release-unit-test', createdAt: new Date().toISOString(), command: argv, toolchain,
    host: { platform: platform(), arch: arch(), hostname: hostname(), cpu: cpus()[0]?.model }, source: before, trackedSourcePatchSha256: sha256(trackedPatch), binary, binarySha256: sha256(await readFile(binary)),
    limitation: 'Optimized engine unit-test executable; not a native-addon or clean-package execution receipt. Local source/binary capture, not independent signed attestation.' };
  await copyFile(join(root, 'woml-cli/scripts/validated-state-baseline.ts'), join(directory, 'harness.ts'));
  await Bun.write(join(directory, 'source.patch'), trackedPatch);
  await Bun.write(join(directory, 'receipt.json'), JSON.stringify(receipt, null, 2));
  console.log(`Build receipt: ${join(directory, 'receipt.json')}`);
}

async function capture(receiptPath: string, trace: boolean) {
  assert.ok(!process.env.WOML_PROFILE && !process.env.WOML_PROFILE_OUTPUT && !process.env.WOML_VSR_CAPTURE, 'Unset profiler/capture variables');
  const receipt = JSON.parse(await readFile(resolve(receiptPath), 'utf8'));
  assert.equal(receipt.profile, 'woml.validated-state-build/v1'); assert.equal(receipt.buildMode, 'release-unit-test');
  assert.equal(receipt.toolchain.bun, Bun.version);
  assert.equal(receipt.host.platform, platform()); assert.equal(receipt.host.arch, arch());
  assert.equal(receipt.host.hostname, hostname()); assert.equal(receipt.host.cpu, cpus()[0]?.model);
  const check = async () => {
    assert.deepEqual(await snapshot(), receipt.source, 'Baseline source/harness changed; build another explicit receipt');
    assert.equal(sha256(await readFile(receipt.binary)), receipt.binarySha256);
    assert.equal(command(['rustc', '--version']).trim(), receipt.toolchain.rustc);
  };
  await check();
  const parent = join(root, 'woml-cli/.woml/performance'); await mkdir(parent, { recursive: true });
  const directory = await mkdtemp(join(parent, trace ? 'validated-state-trace-' : 'validated-state-capture-'));
  await Bun.write(join(directory, 'build-receipt.json'), JSON.stringify(receipt, null, 2));
  await Bun.write(join(directory, 'acceptance.json'), JSON.stringify(acceptance, null, 2));
  for (let index = 0; index < (trace ? 1 : acceptance.processesPerArtifact); index++) {
    const resources = await headroom(); await check();
    const output = join(directory, `process-${index}.json`);
    const tracePath = join(directory, `trace-${index}.jsonl`);
    const child = Bun.spawn([receipt.binary, '--exact', testName, '--ignored', '--nocapture', '--test-threads=1'], { cwd: root,
      env: { ...process.env, WOML_VSR_CAPTURE: output, ...(trace ? { WOML_PROFILE: '1', WOML_PROFILE_OUTPUT: tracePath, WOML_PROFILE_TRACE_ID: `vsr-${index}` } : {}) }, stdout: 'pipe', stderr: 'pipe' });
    const stdout = new Response(child.stdout).text(), stderr = new Response(child.stderr).text();
    const exitCode = await child.exited;
    await Bun.write(join(directory, `process-${index}.log`), `${await stdout}\n${await stderr}`);
    await Bun.write(join(directory, `process-${index}-resources.json`), JSON.stringify({ before: resources, exitCode, usage: resourceRecord(child.resourceUsage()) }, null, 2));
    assert.equal(exitCode, 0, `Capture failed; all raw output retained in ${directory}`);
    validateCapture(JSON.parse(await readFile(output, 'utf8')));
    await check();
  }
  console.log(`Raw ${trace ? 'attribution' : 'baseline'}: ${directory}\nAllocation/peak-live capture and native workflow controls remain required; this alone does not close VSR0.`);
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  if (args.length === 1 && args[0] === 'build') await build();
  else {
    assert.ok((args.length === 3 || (args.length === 4 && args[3] === '--trace')) && args[0] === 'capture' && args[1] === '--receipt' && args[2], 'Use build OR capture --receipt PATH [--trace]');
    await capture(args[2]!, args[3] === '--trace');
  }
}
