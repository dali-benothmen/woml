#!/usr/bin/env bun
// Explicit builds and captures are separate. No install/native override or retries.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { copyFile, mkdir, mkdtemp, readFile } from 'node:fs/promises';
import { arch, cpus, hostname, platform } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import contract from '../../docs/incremental-scheduling-acceptance.v1.json';
import { headroom, resourceRecord, snapshot } from './validated-state-baseline';
import { linuxMaximumRssBytes, median } from './validated-state-comparison';

const root = resolve(import.meta.dir, '../..');
const testName = 'engine::readiness::incremental::capture';
const hash = (value: string | Uint8Array) => createHash('sha256').update(value).digest('hex');
const contractPath = join(root, 'docs/incremental-scheduling-acceptance.v1.json');
const operations = ['historyValidation', 'membership', 'fullQueryIncludingPreparation', 'topologyPreparation'] as const;
type Row = { nodes: number; events: number; sample: number; warmup: boolean; fixtureSha256: string } & Record<typeof operations[number], { wallMs: number; threadCpuMs: number | null }>;
type Capture = { rows: Row[] };

export function withoutIsqTestInstrumentation(source: string) {
  return source.replaceAll('\r\n', '\n').replace(/^[ \t]*#\[cfg\(test\)\]\n[ \t]*(?:mod incremental;|(?:super::)?incremental::note\([^\n]+\);)\n/gm, '');
}

function git(...args: string[]) {
  const result = Bun.spawnSync(['git', ...args], { cwd: root, stdout: 'pipe', stderr: 'pipe' });
  assert.equal(result.exitCode, 0, result.stderr.toString());
  return result.stdout.toString();
}

async function qualifyBaseline() {
  const roots = ['core', 'woml/src', 'woml/tests/fixtures', 'woml-cli/src'];
  const changed = git('diff', '--name-only', 'HEAD', '--', ...roots).trim().split('\n').filter(Boolean).sort();
  assert.deepEqual(changed, ['core/woml-engine/src/engine/readiness.rs', 'core/woml-engine/src/engine/readiness/index.rs']);
  const added = git('ls-files', '--others', '--exclude-standard', '--', ...roots).trim().split('\n').filter(Boolean);
  assert.deepEqual(added, ['core/woml-engine/src/engine/readiness/incremental.rs']);
  for (const path of changed) assert.equal(withoutIsqTestInstrumentation(await readFile(join(root, path), 'utf8')),
    git('show', `${contract.baselineCommit}:${path}`).replaceAll('\r\n', '\n'), `Baseline contains a production change: ${path}`);
}

export function validateIsqCapture(value: unknown): asserts value is Capture {
  assert.ok(value && typeof value === 'object');
  const report = value as Record<string, any>;
  assert.equal(report.profile, 'woml.incremental-scheduling-capture/v1');
  assert.equal(report.allocationMeasurement, null);
  assert.equal(report.peakLiveBytes, null);
  assert.ok(Array.isArray(report.rows));
  const count = contract.warmupSamples + contract.measuredSamples;
  assert.equal(report.rows.length, contract.sizes.length * count);
  const fixtures = new Map<number, string>();
  for (const [index, row] of report.rows.entries()) {
    assert.equal(row.nodes, contract.sizes[Math.floor(index / count)]);
    assert.equal(row.events, row.nodes * 2);
    assert.equal(row.sample, index % count);
    assert.equal(row.warmup, row.sample < contract.warmupSamples);
    assert.match(row.fixtureSha256, /^[0-9a-f]{64}$/);
    if (fixtures.has(row.nodes)) assert.equal(row.fixtureSha256, fixtures.get(row.nodes));
    fixtures.set(row.nodes, row.fixtureSha256);
    for (const operation of operations) {
      const time = row[operation];
      assert.ok(time && Number.isFinite(time.wallMs) && time.wallMs >= 0);
      assert.ok(time.threadCpuMs === null || Number.isFinite(time.threadCpuMs) && time.threadCpuMs >= 0);
    }
  }
}

export function summarizeIsq(baseline: Capture[], candidate: Capture[]) {
  assert.equal(baseline.length, contract.processesPerArtifact);
  assert.equal(candidate.length, contract.processesPerArtifact);
  for (const capture of [...baseline, ...candidate]) {
    validateIsqCapture(capture);
    assert.deepEqual(capture.rows.map(row => row.fixtureSha256), baseline[0]!.rows.map(row => row.fixtureSha256));
  }
  const rows = contract.sizes.flatMap(nodes => operations.flatMap(operation => (['threadCpuMs', 'wallMs'] as const).map(metric => {
    const processMedians = (captures: Capture[]) => captures.map(capture => {
      const values = capture.rows.filter(row => row.nodes === nodes && !row.warmup).map(row => row[operation][metric]);
      return values.includes(null) ? null : median(values as number[]);
    });
    const before = processMedians(baseline), after = processMedians(candidate);
    const a = before.includes(null) ? null : median(before as number[]), b = after.includes(null) ? null : median(after as number[]);
    return { nodes, operation, metric, baselineProcessMedians: before, candidateProcessMedians: after, baseline: a, candidate: b,
      improvementPercent: a === null || b === null || a === 0 ? null : (a - b) / a * 100 };
  })));
  const primary = rows.find(row => row.nodes === contract.primary.nodes && row.operation === contract.primary.operation && row.metric === contract.primary.metric)!;
  return { rows, primary, primaryAccepted: primary.improvementPercent !== null && primary.improvementPercent >= contract.primary.minimumImprovementPercent,
    investigations: rows.filter(row => row.improvementPercent !== null && row.improvementPercent < -contract.investigation.controlMedianRegressionPercent),
    allocationMeasurement: null, peakLiveBytes: null, releaseAccepted: false };
}

async function build(label: string) {
  assert.ok(label === 'baseline' || label === 'candidate');
  if (label === 'baseline') await qualifyBaseline();
  const contractBytes = await readFile(contractPath), harness = await readFile(import.meta.path);
  const child = Bun.spawn([process.execPath, join(import.meta.dir, 'validated-state-baseline.ts'), 'build'], { cwd: root, stdout: 'pipe', stderr: 'inherit' });
  const output = await new Response(child.stdout).text();
  assert.equal(await child.exited, 0, output);
  const receiptPath = output.match(/Build receipt: (.+)/)?.[1];
  assert.ok(receiptPath);
  assert.equal(hash(await readFile(contractPath)), hash(contractBytes));
  assert.equal(hash(await readFile(import.meta.path)), hash(harness));
  const engine = JSON.parse(await readFile(receiptPath, 'utf8'));
  assert.equal(engine.source.commit, contract.baselineCommit, 'Record a reviewed new baseline before changing HEAD');
  const directory = dirname(receiptPath);
  if (label === 'baseline') await qualifyBaseline();
  // Preserve the actual uncommitted sources, not only hashes of a test module
  // which the candidate will later edit. Original committed files stay in Git.
  const sources = engine.source.files.filter((file: { path: string }) => file.path.includes('/engine/readiness') || file.path.includes('/engine/history'));
  for (const file of sources) {
    const destination = join(directory, 'isq-sources', file.path);
    await mkdir(dirname(destination), { recursive: true });
    await copyFile(join(root, file.path), destination);
  }
  const receipt = { profile: 'woml.incremental-scheduling-build/v1', label, engine,
    contractSha256: hash(contractBytes), measurementHarnessSha256: hash(harness) };
  await Bun.write(join(directory, 'isq-contract.json'), contractBytes);
  await Bun.write(join(directory, 'isq-harness.ts'), harness);
  await Bun.write(join(directory, 'isq-receipt.json'), JSON.stringify(receipt, null, 2));
  console.log(`ISQ ${label} receipt: ${join(directory, 'isq-receipt.json')}`);
}

async function capture(receiptPaths: string[]) {
  assert.ok(!['WOML_PROFILE', 'WOML_PROFILE_OUTPUT', 'WOML_ISQ_CAPTURE', 'LD_PRELOAD', 'DYLD_INSERT_LIBRARIES', 'RUST_MIN_STACK'].some(name => process.env[name]), 'Unset tracing, capture, allocator and stack overrides');
  const receipts = await Promise.all(receiptPaths.map(async path => JSON.parse(await readFile(resolve(path), 'utf8'))));
  assert.ok(receipts.length === 1 || receipts.length === 2);
  const harness = await readFile(import.meta.path), contractBytes = await readFile(contractPath);
  for (const [index, receipt] of receipts.entries()) {
    assert.equal(receipt.profile, 'woml.incremental-scheduling-build/v1');
    assert.equal(receipt.contractSha256, hash(contractBytes));
    assert.equal(receipt.measurementHarnessSha256, hash(harness));
    assert.equal(receipt.label, index === 0 ? 'baseline' : 'candidate');
    assert.equal(receipt.engine.source.commit, contract.baselineCommit);
    assert.equal(receipt.engine.profile, 'woml.validated-state-build/v1');
    assert.equal(receipt.engine.buildMode, 'release-unit-test');
    assert.equal(hash(JSON.stringify(receipt.engine.source.files)), receipt.engine.source.sha256);
    assert.deepEqual(receipt.engine.host, { platform: platform(), arch: arch(), hostname: hostname(), cpu: cpus()[0]?.model });
    assert.equal(receipt.engine.toolchain.bun, Bun.version);
    for (const name of ['rustc', 'cargo'] as const) {
      const version = Bun.spawnSync([name, '--version'], { stdout: 'pipe', stderr: 'pipe' });
      assert.equal(version.exitCode, 0); assert.equal(receipt.engine.toolchain[name], version.stdout.toString().trim());
    }
  }
  const check = async () => {
    assert.deepEqual(await snapshot(), receipts.at(-1).engine.source, 'Current source does not match the active receipt');
    assert.equal(hash(await readFile(import.meta.path)), hash(harness));
    assert.equal(hash(await readFile(contractPath)), hash(contractBytes));
    for (const receipt of receipts) assert.equal(hash(await readFile(receipt.engine.binary)), receipt.engine.binarySha256);
  };
  await check();
  const parent = join(root, 'woml-cli/.woml/performance'); await mkdir(parent, { recursive: true });
  const directory = await mkdtemp(join(parent, receipts.length === 1 ? 'isq-baseline-' : 'isq-comparison-'));
  await Bun.write(join(directory, 'inputs.json'), JSON.stringify({ receipts, contract, harnessSha256: hash(harness) }, null, 2));
  await Bun.write(join(directory, 'harness.ts'), harness);
  const reports: Capture[][] = receipts.map(() => []), rss: number[][] = receipts.map(() => []);
  let time: string | null = null;
  if (process.platform === 'linux' && await Bun.file('/usr/bin/time').exists()) {
    const version = Bun.spawnSync(['/usr/bin/time', '--version']);
    assert.equal(version.exitCode, 0); assert.ok(version.stdout.toString().includes('(GNU Time)'));
    time = '/usr/bin/time';
  }
  for (let pair = 0; pair < contract.processesPerArtifact; pair++) {
    const order: number[] = receipts.length === 2 && pair % 2 === 1 ? [1, 0] : receipts.map((_, index) => index);
    for (const index of order) {
      await check(); const resources = await headroom(), receipt = receipts[index]!;
      const output = join(directory, `${pair}-${receipt.label}.json`), rssPath = join(directory, `${pair}-${receipt.label}-rss.txt`);
      const argv = [receipt.engine.binary, '--exact', testName, '--ignored', '--nocapture', '--test-threads=1'];
      const child = Bun.spawn(time ? [time, '-f', '%M', '-o', rssPath, ...argv] : argv, { cwd: root,
        env: { ...process.env, WOML_ISQ_CAPTURE: output }, stdout: 'pipe', stderr: 'pipe', timeout: 300_000 });
      const stdout = new Response(child.stdout).text(), stderr = new Response(child.stderr).text(), exitCode = await child.exited;
      await Bun.write(join(directory, `${pair}-${receipt.label}.log`), `${await stdout}\n${await stderr}`);
      await Bun.write(join(directory, `${pair}-${receipt.label}-resources.json`), JSON.stringify({ before: resources, pair, order, exitCode, usage: resourceRecord(child.resourceUsage()) }, null, 2));
      assert.equal(exitCode, 0, `Failed capture retained in ${directory}`);
      const report = JSON.parse(await readFile(output, 'utf8')); validateIsqCapture(report); reports[index]!.push(report);
      if (time) rss[index]!.push(linuxMaximumRssBytes(await readFile(rssPath, 'utf8')));
      await check(); console.log(`ISQ pair ${pair + 1}/${contract.processesPerArtifact}: ${receipt.label}`);
    }
  }
  const summary = receipts.length === 2 ? summarizeIsq(reports[0]!, reports[1]!) : { baselineOnly: true, releaseAccepted: false };
  const memory = time ? rss.map(values => ({ processMaximumRssBytes: values, medianBytes: median(values) })) : null;
  await Bun.write(join(directory, 'summary.json'), JSON.stringify({ ...summary, wholeProcessMaximumRss: memory }, null, 2));
  console.log(`ISQ raw capture: ${directory}\n${JSON.stringify(summary, null, 2)}`);
}

if (import.meta.main) {
  const args = process.argv.slice(2);
  if (args.length === 2 && args[0] === 'build') await build(args[1]!);
  else if (args.length === 3 && args[0] === 'baseline' && args[1] === '--receipt') await capture([args[2]!]);
  else {
    assert.ok(args.length === 5 && args[0] === 'compare' && args[1] === '--baseline' && args[3] === '--candidate', 'Use build baseline|candidate; baseline --receipt PATH; compare --baseline PATH --candidate PATH');
    await capture([args[2]!, args[4]!]);
  }
}
