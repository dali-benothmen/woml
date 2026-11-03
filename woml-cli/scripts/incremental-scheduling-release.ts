#!/usr/bin/env bun
// Historical-baseline continuation, not a replacement for the frozen runner.
// Capture never builds, installs, retries, or certifies release acceptance.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile } from 'node:fs/promises';
import { arch, cpus, hostname, platform, release } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import contract from '../../docs/incremental-scheduling-acceptance.v1.json';
import { summarizeIsq, validateIsqCapture } from './incremental-scheduling-measurement';
import { headroom, resourceRecord, snapshot } from './validated-state-baseline';
import { linuxMaximumRssBytes, median } from './validated-state-comparison';

const root = resolve(import.meta.dir, '../..');
const sha = (value: string | Uint8Array) => createHash('sha256').update(value).digest('hex');
// Exact originally qualified ISQ0 artifact, not an arbitrary receipt labelled
// "baseline". Keep the original receipt, test adapter and qualification intact.
const original = {
  binary: 'dcd04b1db65e2153ae441531273f09aad627d67eaed4d334a43d80dca02b9b15',
  source: '3cb433f0f405625da1125664752b46ea075ccdba7bf7cbbd308c300d75a5a0f7',
  harness: '7caabde8507ed203053e4aaa4ec88d74a70d2294ab4008b3f69f1869cc0b5bcb',
};
const probePath = 'core/woml-engine/src/engine/readiness/incremental.rs';
const harnessPaths = [
  'woml-cli/scripts/incremental-scheduling-release.ts',
  'woml-cli/scripts/incremental-scheduling-measurement.ts',
  'woml-cli/scripts/validated-state-baseline.ts',
  'woml-cli/scripts/validated-state-comparison.ts',
  'docs/incremental-scheduling-acceptance.v1.json',
];

export function assertOriginalIsqIdentity(receipt: any) {
  assert.equal(receipt.profile, 'woml.incremental-scheduling-build/v1');
  assert.equal(receipt.label, 'baseline');
  assert.equal(receipt.engine.profile, 'woml.validated-state-build/v1');
  assert.equal(receipt.engine.buildMode, 'release-unit-test');
  assert.equal(receipt.engine.source.commit, contract.baselineCommit);
  assert.equal(receipt.engine.source.sha256, original.source);
  assert.equal(receipt.engine.binarySha256, original.binary);
  assert.equal(receipt.measurementHarnessSha256, original.harness);
}

export function captureBody(source: string) {
  // The frozen probe is the final item in this private module. Refuse later
  // adapter edits rather than silently comparing different workloads.
  const marker = '\nfn capture() {';
  assert.equal(source.split(marker).length, 2, 'Missing or duplicate frozen capture');
  return source.slice(source.indexOf(marker));
}

export function releaseMicrobenchmarkEvidence(
  before: Parameters<typeof summarizeIsq>[0], after: Parameters<typeof summarizeIsq>[1],
  maximumRss: number[][] | null,
) {
  const result = summarizeIsq(before, after);
  if (maximumRss !== null) {
    assert.equal(maximumRss.length, 2);
    for (const values of maximumRss) {
      assert.equal(values.length, contract.processesPerArtifact);
      for (const value of values) assert.ok(Number.isSafeInteger(value) && value > 0);
    }
  }
  const wholeProcessMaximumRss = maximumRss?.map(values => ({ processBytes: values, medianBytes: median(values) })) ?? null;
  const rssIncreaseBytes = wholeProcessMaximumRss === null ? null
    : wholeProcessMaximumRss[1]!.medianBytes - wholeProcessMaximumRss[0]!.medianBytes;
  return {
    ...result, profile: 'woml.isq-release-microbenchmark/v1',
    status: 'microbenchmark-only', releaseAccepted: false,
    wholeProcessMaximumRss, rssIncreaseBytes,
    rssInvestigationRequired: rssIncreaseBytes === null ? null
      : rssIncreaseBytes > contract.investigation.rssOrScopedPeakLiveIncreaseBytes,
    // No process-RSS observation may stand in for scoped allocation or native
    // evidence. Even a numeric primary pass cannot waive these prerequisites.
    releasePrerequisitesNotCertifiedByThisCapture: contract.releasePrerequisites,
  };
}

async function main() {
  const args = process.argv.slice(2);
  assert.ok(args.length === 4 && args[0] === '--baseline' && args[2] === '--candidate',
    'Use --baseline ORIGINAL_ISQ_RECEIPT --candidate CURRENT_ENGINE_RECEIPT; build explicitly first');
  for (const name of ['WOML_PROFILE', 'WOML_PROFILE_OUTPUT', 'WOML_ISQ_CAPTURE', 'LD_PRELOAD',
    'DYLD_INSERT_LIBRARIES', 'RUST_MIN_STACK', 'WOML_ISQ_CONTROL_MODE', 'WOML_ISQ_REUSE_RESOURCES']) {
    assert.ok(!process.env[name], `Unset ${name}`);
  }
  const baselinePath = resolve(args[1]!), candidatePath = resolve(args[3]!);
  const historical = JSON.parse(await readFile(baselinePath, 'utf8'));
  assertOriginalIsqIdentity(historical);
  const baseline = historical.engine;
  const candidate = JSON.parse(await readFile(candidatePath, 'utf8'));
  assert.equal(candidate.profile, 'woml.validated-state-build/v1');
  assert.equal(candidate.buildMode, 'release-unit-test');
  assert.notEqual(candidate.binarySha256, baseline.binarySha256, 'Cannot compare the baseline with itself');
  const harness = await Promise.all(harnessPaths.map(async path => ({ path, sha256: sha(await readFile(join(root, path))) })));
  const archivedContract = await readFile(join(dirname(baselinePath), 'isq-contract.json'));
  assert.equal(sha(archivedContract), historical.contractSha256);
  assert.equal(sha(await readFile(join(root, 'docs/incremental-scheduling-acceptance.v1.json'))), historical.contractSha256);
  assert.equal(sha(await readFile(join(dirname(baselinePath), 'isq-harness.ts'))), original.harness);
  // Check archived sources against the pinned manifest, not just receipt labels.
  const archivedSources = baseline.source.files.filter((file: { path: string }) =>
    file.path.includes('/engine/readiness') || file.path.includes('/engine/history'));
  assert.ok(archivedSources.some((file: { path: string }) => file.path === probePath));
  for (const file of archivedSources) {
    assert.equal(sha(await readFile(join(dirname(baselinePath), 'isq-sources', file.path))), file.sha256);
  }
  assert.equal(captureBody(await readFile(join(root, probePath), 'utf8')),
    captureBody(await readFile(join(dirname(baselinePath), 'isq-sources', probePath), 'utf8')),
    'The frozen probe body changed; qualify a separate diagnostic instead');
  const receipts = [baseline, candidate];
  for (const receipt of receipts) {
    assert.equal(sha(JSON.stringify(receipt.source.files)), receipt.source.sha256);
    assert.deepEqual(receipt.host, { platform: platform(), arch: arch(), hostname: hostname(), cpu: cpus()[0]?.model });
    assert.equal(receipt.toolchain.bun, Bun.version);
    for (const name of ['rustc', 'cargo']) {
      const version = Bun.spawnSync([name, '--version'], { stdout: 'pipe', stderr: 'pipe' });
      assert.equal(version.exitCode, 0); assert.equal(receipt.toolchain[name], version.stdout.toString().trim());
    }
  }
  const check = async () => {
    assert.deepEqual(await snapshot(), candidate.source, 'Current source/HEAD changed; rebuild explicitly');
    for (const file of harness) assert.equal(sha(await readFile(join(root, file.path))), file.sha256);
    for (const receipt of receipts) assert.equal(sha(await readFile(receipt.binary)), receipt.binarySha256);
  };
  await check(); await headroom();
  const parent = join(root, 'woml-cli/.woml/performance'); await mkdir(parent, { recursive: true });
  const directory = await mkdtemp(join(parent, 'isq-release-'));
  for (const file of harness) {
    const destination = join(directory, 'harness', file.path);
    await mkdir(dirname(destination), { recursive: true });
    await Bun.write(destination, await readFile(join(root, file.path)));
  }
  await Bun.write(join(directory, 'inputs.json'), JSON.stringify({ historical, candidate, harness, contract,
    hostKernel: release(), purpose: 'Original qualified ISQ0 versus current candidate, frozen probe; not release acceptance',
    order: 'seven alternating adjacent process pairs; one warmup and three measured samples per size',
  }, null, 2));
  const reports: Parameters<typeof summarizeIsq>[0][] = [[], []];
  const rss: number[][] = [[], []];
  let time: string | null = null;
  if (process.platform === 'linux' && await Bun.file('/usr/bin/time').exists()) {
    const version = Bun.spawnSync(['/usr/bin/time', '--version'], { stdout: 'pipe', stderr: 'pipe' });
    assert.equal(version.exitCode, 0); assert.ok(version.stdout.toString().includes('(GNU Time)'));
    time = '/usr/bin/time';
  }
  for (let pair = 0; pair < contract.processesPerArtifact; pair++) {
    for (const index of pair % 2 ? [1, 0] : [0, 1]) {
      await check(); const before = await headroom();
      const output = join(directory, `${pair}-${index}.json`), rssPath = output + '.rss-kib.txt';
      const argv = [receipts[index].binary, '--exact', 'engine::readiness::incremental::capture', '--ignored', '--nocapture', '--test-threads=1'];
      const child = Bun.spawn(time ? [time, '-f', '%M', '-o', rssPath, ...argv] : argv, {
        cwd: root, env: { ...process.env, WOML_ISQ_CAPTURE: output }, stdout: 'pipe', stderr: 'pipe', timeout: 300_000,
      });
      const stdout = new Response(child.stdout).text(), stderr = new Response(child.stderr).text(), exitCode = await child.exited;
      await Bun.write(output + '.log', `${await stdout}\n${await stderr}`);
      await Bun.write(output + '.resources.json', JSON.stringify({ before, exitCode, usage: resourceRecord(child.resourceUsage()) }, null, 2));
      assert.equal(exitCode, 0, `Failed capture retained at ${directory}; do not retry into acceptance`);
      const report = JSON.parse(await readFile(output, 'utf8')); validateIsqCapture(report);
      reports[index]!.push(report);
      if (time) rss[index]!.push(linuxMaximumRssBytes(await readFile(rssPath, 'utf8')));
      await check(); console.log(`ISQ6 pair ${pair + 1}/${contract.processesPerArtifact}: ${index === 0 ? 'original' : 'candidate'}`);
    }
  }
  const summary = releaseMicrobenchmarkEvidence(reports[0]!, reports[1]!, time ? rss : null);
  await Bun.write(join(directory, 'summary.json'), JSON.stringify(summary, null, 2));
  console.log(`ISQ6 evidence: ${directory}\nNumeric primary passed: ${summary.primaryAccepted}; release accepted: false`);
}
if (import.meta.main) await main();
