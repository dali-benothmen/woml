#!/usr/bin/env bun
// Explicit optimized receipt only. No builds, installs, native selection or gate changes.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile, statfs } from 'node:fs/promises';
import { arch, cpus, freemem, hostname, platform } from 'node:os';
import { join, resolve } from 'node:path';
import { resourceRecord } from './validated-state-baseline';

const root = resolve(import.meta.dir, '../..');
const sizes = [1, 4, 16, 64, 256, 1024];
const testName = 'engine::readiness::tests::capture_indexed';
const hash = (value: Uint8Array | string) => createHash('sha256').update(value).digest('hex');

export function validateIndexCapture(value: unknown): asserts value is { rows: Record<string, any>[] } {
  assert.ok(value && typeof value === 'object');
  const report = value as Record<string, any>;
  assert.equal(report.profile, 'woml.readiness-indexes/v1');
  assert.equal(report.allocationMeasurement, null);
  assert.equal(report.peakLiveBytes, null);
  assert.ok(Array.isArray(report.rows));
  assert.equal(report.rows.length, sizes.length * 4);
  const fixtures = new Map<number, string>();
  for (const [position, row] of report.rows.entries()) {
    assert.equal(row.nodes, sizes[Math.floor(position / 4)]);
    assert.equal(row.events, row.nodes * 2);
    assert.equal(row.sample, position % 4);
    assert.equal(row.warmup, row.sample === 0);
    assert.match(row.fixtureSha256, /^[a-f0-9]{64}$/);
    if (fixtures.has(row.nodes)) assert.equal(row.fixtureSha256, fixtures.get(row.nodes));
    fixtures.set(row.nodes, row.fixtureSha256);
    for (const name of ['scanValidation', 'indexedValidation', 'scanQuery', 'indexedQueryIncludingPreparation', 'graphPreparation']) {
      const clock = row[name];
      assert.ok(clock && Number.isFinite(clock.wallMs) && clock.wallMs >= 0);
      assert.ok(clock.threadCpuMs === null || Number.isFinite(clock.threadCpuMs) && clock.threadCpuMs >= 0);
    }
  }
}

async function main() {
  const args = process.argv.slice(2);
  assert.ok(args.length === 2 && args[0] === '--receipt', 'Use --receipt PATH from build:validated-state-reuse-baseline; capture never builds');
  for (const name of ['WOML_PROFILE', 'WOML_PROFILE_OUTPUT', 'WOML_READINESS_INDEX_CAPTURE']) assert.ok(!process.env[name], `Unset ${name}`);
  const receipt = JSON.parse(await readFile(resolve(args[1]!), 'utf8'));
  assert.equal(receipt.profile, 'woml.validated-state-build/v1');
  assert.equal(receipt.buildMode, 'release-unit-test');
  assert.deepEqual(receipt.host, { platform: platform(), arch: arch(), hostname: hostname(), cpu: cpus()[0]?.model });
  assert.equal(receipt.toolchain.bun, Bun.version);
  const harness = await readFile(import.meta.path);
  const check = async () => {
    const commit = Bun.spawnSync(['git', 'rev-parse', 'HEAD'], { cwd: root });
    assert.equal(commit.exitCode, 0);
    assert.equal(commit.stdout.toString().trim(), receipt.source.commit);
    assert.equal(hash(await readFile(receipt.binary)), receipt.binarySha256);
    assert.equal(hash(await readFile(import.meta.path)), hash(harness));
    const rustc = Bun.spawnSync(['rustc', '--version']);
    assert.equal(rustc.exitCode, 0);
    assert.equal(rustc.stdout.toString().trim(), receipt.toolchain.rustc);
    const inventory = Bun.spawnSync(['git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z', '--', 'core', 'woml/src', 'woml/tests/fixtures', 'woml-cli/src'], { cwd: root });
    assert.equal(inventory.exitCode, 0);
    const expected = new Set<string>(receipt.source.files.map((file: { path: string }) => file.path));
    for (const path of inventory.stdout.toString().split('\0').filter(Boolean)) assert.ok(expected.has(path), `Unreceipted source: ${path}`);
    for (const file of receipt.source.files) assert.equal(hash(await readFile(join(root, file.path))), file.sha256, `Changed source: ${file.path}`);
  };
  await check();
  const parent = join(root, 'woml-cli/.woml/performance');
  await mkdir(parent, { recursive: true });
  const directory = await mkdtemp(join(parent, 'readiness-indexes-'));
  await Bun.write(join(directory, 'build-receipt.json'), JSON.stringify(receipt, null, 2));
  await Bun.write(join(directory, 'harness.ts'), harness);
  await Bun.write(join(directory, 'protocol.json'), JSON.stringify({
    profile: 'woml.readiness-index-protocol/v1', harnessSha256: hash(harness), processes: 3, sizes,
    warmups: 1, measuredSamples: 3, order: 'alternating-scan-indexed',
    validationOutsideTiming: true, queryIncludesPreparation: true, referenceUsesMonotoneCursor: true,
    replacementForVsrAcceptance: false,
  }, null, 2));
  const reports: { rows: Record<string, any>[] }[] = [];
  for (let index = 0; index < 3; index++) {
    await check();
    const memory = process.platform === 'linux' ? Number((await readFile('/proc/meminfo', 'utf8')).match(/^MemAvailable:\s+(\d+) kB$/m)?.[1] ?? 0) * 1024 : freemem();
    const disk = await statfs(root);
    assert.ok(memory >= 2 ** 31 && disk.bavail * disk.bsize >= 2 ** 31, 'Need at least 2 GiB available RAM/disk');
    const output = join(directory, `process-${index}.json`);
    const child = Bun.spawn([receipt.binary, '--exact', testName, '--ignored', '--nocapture', '--test-threads=1'], {
      cwd: root, env: { ...process.env, WOML_READINESS_INDEX_CAPTURE: output }, stdout: 'pipe', stderr: 'pipe', timeout: 300_000,
    });
    const stdout = new Response(child.stdout).text(), stderr = new Response(child.stderr).text();
    const exitCode = await child.exited;
    await Bun.write(join(directory, `process-${index}.log`), `${await stdout}\n${await stderr}`);
    await Bun.write(join(directory, `process-${index}-resources.json`), JSON.stringify({ memoryBeforeBytes: memory, exitCode, usage: resourceRecord(child.resourceUsage()) }, null, 2));
    assert.equal(exitCode, 0, `Failed paired capture retained in ${directory}`);
    const report = JSON.parse(await readFile(output, 'utf8'));
    validateIndexCapture(report);
    reports.push(report);
    await check();
    console.log(`Readiness index capture ${index + 1}/3 complete`);
  }
  for (const nodes of sizes) {
    const identities = new Set<string>(reports.flatMap(report => report.rows.filter(row => row.nodes === nodes).map(row => row.fixtureSha256 as string)));
    assert.equal(identities.size, 1, 'Fixture drift between processes');
  }
  console.log(`Raw paired capture: ${directory}`);
}

if (import.meta.main) await main();
