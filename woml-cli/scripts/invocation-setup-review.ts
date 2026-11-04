#!/usr/bin/env bun
// ISR5 offline review: never reruns a timing, edits raw evidence or accepts a release.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import acceptance from '../../docs/invocation-setup-acceptance.v1.json';
import prototype from '../../docs/invocation-facade-prototype.v1.json';
import { encodeFrame } from '../src/script-host/framing';
import { cohorts, fixture, sha256 } from './invocation-setup-fixtures';
import { summarizeFacade } from './invocation-facade-prototype';

export function distribution(values: number[]) {
  assert.ok(values.length > 0 && values.every(Number.isFinite));
  const sorted = [...values].sort((a, b) => a - b), middle = Math.floor(sorted.length / 2);
  return { count: values.length, minimum: sorted[0]!, median: sorted.length % 2 ? sorted[middle]! : (sorted[middle - 1]! + sorted[middle]!) / 2,
    maximum: sorted.at(-1)!, values: [...values] };
}
export function breakEven(extraSetupMs: number, savedBatchMs: number) {
  assert.ok(Number.isFinite(extraSetupMs) && Number.isFinite(savedBatchMs));
  return savedBatchMs <= 0 ? { observedFiniteBreakEven: false, hypotheticalBatches: null }
    : { observedFiniteBreakEven: null, hypotheticalBatches: Math.max(1, Math.ceil(Math.max(0, extraSetupMs) / savedBatchMs)),
      limitation: 'Aggregate-median cost model, not a measured reuse-count crossing' };
}
function volumes(cohort: string) {
  const spec = fixture(cohort);
  let requests = 0, requestDirectionFramedBytes = spec.artifacts.reduce((sum, artifact) => sum + encodeFrame(artifact).length, 0);
  let workerDeliveredBundleUtf8Bytes = 0, workerDeliveredMapUtf8Bytes = 0;
  for (const batch of ['first', ...Array.from({ length: acceptance.warmupBatches + acceptance.measuredBatches }, (_, i) => `sample${i}`)]) {
    for (let index = 0; index < (batch === 'first' ? 1 : spec.count); index++) {
      const request = spec.request(index, batch); requests++; requestDirectionFramedBytes += encodeFrame(request).length;
      for (const module of request.modules) {
        const artifact = spec.artifacts.find(artifact => artifact.bundleDigest === module.bundleDigest)!; assert.ok(artifact);
        workerDeliveredBundleUtf8Bytes += Buffer.byteLength(artifact.bundle);
        workerDeliveredMapUtf8Bytes += Buffer.byteLength(artifact.sourceMap);
      }
    }
  }
  return { requests, requestDirectionFramedBytes, workerDeliveredBundleUtf8Bytes, workerDeliveredMapUtf8Bytes,
    registeredArtifactUtf8Bytes: spec.artifacts.reduce((sum, item) => sum + Buffer.byteLength(item.bundle) + Buffer.byteLength(item.sourceMap), 0),
    actualStructuredCloneAllocatedBytes: null,
    scope: 'Reconstructed from unchanged exact fixtures, including first requests/warmups. UTF-8 module bytes are logical delivered content, not structured-clone allocation or physical resident memory.' };
}
export function reviewInvocationEvidence(reference: any[], candidate: any[]) {
  const comparison = summarizeFacade(reference, candidate);
  const rows = cohorts.map(cohort => {
    const before = reference.map(report => report.rows.find((row: any) => row.cohort === cohort));
    const after = candidate.map(report => report.rows.find((row: any) => row.cohort === cohort));
    for (const row of [...before, ...after]) {
      const accounted = row.readyMs + row.registrationMs + row.firstInvocationMs + row.teardownMs + row.batches.reduce((sum: number, batch: any) => sum + batch.wallMs, 0);
      assert.ok(row.lifetimeMs + 0.01 >= accounted, 'Overlapping/impossible lifetime accounting');
    }
    const batchMedian = (row: any) => distribution(row.batches.filter((batch: any) => !batch.warmup).map((batch: any) => batch.wallMs)).median;
    const pairChanges = before.map((row, i) => (batchMedian(after[i]) / batchMedian(row) - 1) * 100);
    const metric = (name: string) => ({ name, reference: distribution(before.map(row => row[name])), candidate: distribution(after.map(row => row[name])) });
    return { cohort, metrics: ['readyMs', 'registrationMs', 'firstInvocationMs', 'teardownMs', 'lifetimeMs'].map(metric),
      coldSetupMs: { reference: distribution(before.map(row => row.readyMs + row.registrationMs + row.firstInvocationMs)),
        candidate: distribution(after.map(row => row.readyMs + row.registrationMs + row.firstInvocationMs)) },
      warmupBatchMs: { reference: distribution(before.map(row => row.batches[0].wallMs)), candidate: distribution(after.map(row => row.batches[0].wallMs)) },
      warmBatchMs: { reference: distribution(before.map(batchMedian)), candidate: distribution(after.map(batchMedian)) },
      pairedRegressionPercent: distribution(pairChanges), candidateSlowerPairs: pairChanges.filter(value => value > 0).length,
      rawHostResourceObservations: { reference: before.map(row => row.hostProcessUsage ?? null), candidate: after.map(row => row.hostProcessUsage ?? null) },
      volumesPerHostBothVariants: volumes(cohort),
      controlInvestigation: comparison.controlInvestigations.includes(cohort)
        ? 'Triggered. Fixture and completion equivalence verified; paired/whole-lifetime distributions retained. Causal slowdown not isolated; candidate rejected, not excused as noise.' : null };
  });
  const primary = rows.find(row => row.cohort === acceptance.primary.cohort)!;
  return { profile: 'woml.invocation-setup-review/v1', comparison, rows,
    primaryBreakEven: breakEven(primary.coldSetupMs.candidate.median - primary.coldSetupMs.reference.median, primary.warmBatchMs.reference.median - primary.warmBatchMs.candidate.median),
    unmeasured: { scopedCpuMs: null, allocatedBytes: null, activeWorkerPeakBytes: null, retainedHostHeapBytes: null, repeatedRunPlateau: null, actualCloneBytes: null, rssIncreaseBytes: null },
    rssTriggerEvaluable: false,
    decision: comparison.numericalPrimaryTargetMet
      ? 'Numerical target met only; integration requires independent safety/resource/native acceptance.'
      : 'Retain production worker. Candidate remains diagnostic-only; numerical primary target failed.',
    releaseAccepted: false, integrationAccepted: false };
}

export async function auditPrototypeReceipt(receipt: any) {
  assert.equal(receipt.profile, 'woml.invocation-facade-build/v1'); assert.equal(receipt.releaseAccepted, false);
  assert.equal(receipt.baseline.inputs.source.commit, acceptance.baselineCommit);
  assert.deepEqual(receipt.inputs.source.files, receipt.baseline.inputs.source.files);
  assert.equal(sha256(JSON.stringify(receipt.inputs.source.files)), receipt.inputs.source.sha256);
  const names = ['script-host.js', 'script-host-worker.js', 'script-host.js.map', 'script-host-worker.js.map'];
  assert.deepEqual(receipt.artifacts.map((file: any) => `${file.variant}/${file.name}`).sort(), ['reference', 'candidate'].flatMap(variant => names.map(name => `${variant}/${name}`)).sort());
  for (const file of receipt.artifacts) {
    const bytes = await readFile(file.path); assert.equal(sha256(bytes), file.sha256); assert.equal(bytes.length, file.bytes);
    if (file.name === 'script-host.js') assert.equal(file.sha256, prototype.originalHostSha256);
    if (file.variant === 'reference') {
      const original = receipt.baseline.artifacts.find((item: any) => item.name === file.name); assert.ok(original); assert.equal(original.sha256, file.sha256);
      assert.equal(sha256(await readFile(original.path)), original.sha256);
    }
    if (file.variant === 'reference' && file.name === 'script-host-worker.js') assert.equal(file.sha256, prototype.originalWorkerSha256);
  }
  const archive = dirname(dirname(receipt.artifacts[0].path));
  for (const file of receipt.inputs.harness) assert.equal(sha256(await readFile(join(archive, 'harness', file.path))), file.sha256);
  assert.equal(sha256(await readFile(join(archive, 'original-worker.ts'))), prototype.originalWorkerSourceSha256);
  assert.equal(sha256(await readFile(join(archive, 'prototype-worker.ts'))), receipt.transformedSourceSha256);
  return { archive, artifactsVerified: receipt.artifacts.length, harnessFilesVerified: receipt.inputs.harness.length };
}

export async function analyzeInvocationCapture(directory: string) {
  const read = async (path: string) => JSON.parse(await readFile(path, 'utf8'));
  const input = await read(join(directory, 'inputs.json'));
  assert.deepEqual(input.acceptance, acceptance); assert.deepEqual(input.contract, prototype);
  const provenance = await auditPrototypeReceipt(input.receipt);
  const reports: Record<'reference' | 'candidate', any[]> = { reference: [], candidate: [] }, rawFiles = [];
  for (let i = 0; i < acceptance.processesPerArtifact; i++) for (const variant of ['reference', 'candidate'] as const) {
    const path = join(directory, `pair${i}-${variant}`), resources = await read(join(path, 'resources.json'));
    assert.equal(resources.exitCode, 0); reports[variant].push(await read(join(path, 'capture.json')));
    for (const name of ['capture.json', 'resources.json', 'process.log']) rawFiles.push({ path: join(path, name), sha256: sha256(await readFile(join(path, name))) });
  }
  const result = reviewInvocationEvidence(reports.reference, reports.candidate);
  assert.deepEqual(result.comparison, await read(join(directory, 'summary.json')), 'Original summary drift');
  return { ...result, provenance, rawFiles, inputSha256: sha256(await readFile(join(directory, 'inputs.json'))), analyzerSha256: sha256(await readFile(import.meta.path)) };
}
if (import.meta.main) {
  assert.equal(process.argv.length, 3, 'Use ORIGINAL_MATCHED_CAPTURE_DIRECTORY');
  const directory = resolve(process.argv[2]!), output = join(directory, 'isr5-review.json');
  assert.equal(await Bun.file(output).exists(), false, 'Never overwrite an earlier review');
  const report = await analyzeInvocationCapture(directory);
  await Bun.write(output, JSON.stringify(report, null, 2)); console.log(`ISR5 review: ${output}`);
}
