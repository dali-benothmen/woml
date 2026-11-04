#!/usr/bin/env bun
// New exploratory diagnosis, NOT a replacement for VSR0's acceptance cohort.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile, statfs } from 'node:fs/promises';
import { arch, cpus, freemem, hostname, platform } from 'node:os';
import { join, resolve } from 'node:path';
import { resourceRecord } from './validated-state-baseline';

const root = resolve(import.meta.dir, '../..');
const hash = (bytes: string | Uint8Array) => createHash('sha256').update(bytes).digest('hex');
const sizes = [16, 64, 256, 1024];
const testName = 'engine::readiness::investigation::capture';

export function validateReadinessCapture(report: any) {
  assert.equal(report?.profile, 'woml.readiness-investigation/v1');
  assert.ok(Array.isArray(report.components)); assert.equal(report.components.length, 60);
  assert.ok(Array.isArray(report.histories)); assert.equal(report.histories.length, 8);
  const clock = (value: any) => {
    assert.ok(value && Number.isFinite(value.wallMs) && value.wallMs >= 0);
    assert.ok(value.threadCpuMs === null || Number.isFinite(value.threadCpuMs) && value.threadCpuMs >= 0);
  };
  const identities = new Map<number, string>();
  for (const [index, row] of report.components.entries()) {
    const nodes = sizes[Math.floor(index / 15)]!;
    assert.equal(row.nodes, nodes); assert.equal(row.completed, [0, nodes / 2, nodes - 1][Math.floor(index / 5) % 3]);
    assert.equal(row.historyEvents, 1 + 2 * row.completed);
    assert.equal(row.sample, index % 5); assert.equal(row.warmup, index % 5 < 2);
    assert.match(row.fixtureSha256, /^[a-f0-9]{64}$/);
    if (identities.has(nodes)) assert.equal(row.fixtureSha256, identities.get(nodes));
    identities.set(nodes, row.fixtureSha256);
    clock(row.wholeQuery);
    for (const phase of ['activeTraversal', 'incomingConstruction', 'nodeFiltering']) clock(row.stages?.[phase]);
    assert.equal(row.countedInspections?.outgoingEdgeInspections, nodes * (nodes - 1));
    assert.equal(row.countedInspections?.latestAttemptInspections, (nodes - row.completed) * row.completed);
    let expectedNodeTests = 0;
    for (let position = Math.max(row.completed, 1); position < nodes; position++) expectedNodeTests += position;
    assert.equal(row.countedInspections?.predecessorNodeInspections, expectedNodeTests);
  }
  for (const [index, row] of report.histories.entries()) {
    const nodes = sizes[Math.floor(index / 2)]!;
    assert.equal(row.nodes, nodes); assert.equal(row.cohort, index % 2 === 0 ? 'full-active' : 'fixed-17-events');
    assert.equal(row.events, index % 2 === 0 ? nodes * 2 : 17);
    assert.equal(row.queries, index % 2 === 0 ? nodes : 8);
    assert.equal(row.prefixVisits, row.events); assert.equal(row.fixtureSha256, identities.get(nodes));
    for (const name of ['wholeValidation', 'payloadChecks', 'structuralFold', 'readinessAndPrefixReplay']) clock(row[name]);
  }
}

async function main() {
  const args = process.argv.slice(2);
  assert.ok(args.length === 2 && args[0] === '--receipt', 'Use --receipt PATH from the explicit baseline build; capture never builds');
  for (const name of ['WOML_PROFILE', 'WOML_PROFILE_OUTPUT', 'WOML_READINESS_CAPTURE']) assert.ok(!process.env[name], `Unset ${name}`);
  const receipt = JSON.parse(await readFile(resolve(args[1]!), 'utf8'));
  assert.equal(receipt.profile, 'woml.validated-state-build/v1'); assert.equal(receipt.buildMode, 'release-unit-test');
  assert.deepEqual(receipt.host, { platform: platform(), arch: arch(), hostname: hostname(), cpu: cpus()[0]?.model });
  assert.equal(receipt.toolchain.bun, Bun.version);
  const harness = await readFile(import.meta.path);
  const check = async () => {
    const commit = Bun.spawnSync(['git', 'rev-parse', 'HEAD'], { cwd: root });
    assert.equal(commit.exitCode, 0); assert.equal(commit.stdout.toString().trim(), receipt.source.commit);
    assert.equal(hash(await readFile(receipt.binary)), receipt.binarySha256);
    assert.equal(hash(await readFile(import.meta.path)), hash(harness));
    const rustc = Bun.spawnSync(['rustc', '--version']);
    assert.equal(rustc.exitCode, 0); assert.equal(rustc.stdout.toString().trim(), receipt.toolchain.rustc);
    for (const file of receipt.source.files) assert.equal(hash(await readFile(join(root, file.path))), file.sha256, `Changed source: ${file.path}`);
  };
  await check();
  const parent = join(root, 'woml-cli/.woml/performance'); await mkdir(parent, { recursive: true });
  const directory = await mkdtemp(join(parent, 'readiness-investigation-'));
  await Bun.write(join(directory, 'build-receipt.json'), JSON.stringify(receipt, null, 2));
  await Bun.write(join(directory, 'harness.ts'), harness);
  await Bun.write(join(directory, 'protocol.json'), JSON.stringify({ profile: 'woml.readiness-investigation-protocol/v1', harnessSha256: hash(harness), processes: 3, sizes, completed: ['zero', 'half', 'last'], warmups: 2, measuredSamples: 3, queryOrder: 'alternating-production-staged', historySamplesPerProcess: 1, replacementForVsrAcceptance: false }, null, 2));
  for (let index = 0; index < 3; index++) {
    await check();
    const memory = process.platform === 'linux' ? Number((await readFile('/proc/meminfo', 'utf8')).match(/^MemAvailable:\s+(\d+) kB$/m)?.[1] ?? 0) * 1024 : freemem();
    const disk = await statfs(root);
    assert.ok(memory >= 2 ** 31 && disk.bavail * disk.bsize >= 2 ** 31, 'Need at least 2 GiB available RAM/disk');
    const output = join(directory, `process-${index}.json`);
    const child = Bun.spawn([receipt.binary, '--exact', testName, '--ignored', '--nocapture', '--test-threads=1'], {
      cwd: root, env: { ...process.env, WOML_READINESS_CAPTURE: output }, stdout: 'pipe', stderr: 'pipe', timeout: 300_000,
    });
    const stdout = new Response(child.stdout).text(), stderr = new Response(child.stderr).text();
    const exitCode = await child.exited;
    await Bun.write(join(directory, `process-${index}.log`), `${await stdout}\n${await stderr}`);
    await Bun.write(join(directory, `process-${index}-resources.json`), JSON.stringify({ memoryBeforeBytes: memory, exitCode, usage: resourceRecord(child.resourceUsage()) }, null, 2));
    assert.equal(exitCode, 0, `Failed diagnostic capture retained in ${directory}`);
    validateReadinessCapture(JSON.parse(await readFile(output, 'utf8')));
    await check();
    console.log(`Readiness diagnostic ${index + 1}/3 complete`);
  }
  console.log(`Raw diagnosis: ${directory}`);
}
if (import.meta.main) await main();
