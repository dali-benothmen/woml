#!/usr/bin/env bun
// Offline diagnosis only. Never edits raw capture or converts a baseline to a pass.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import contract from '../../docs/invocation-setup-acceptance.v1.json';
import { cohorts, sha256 } from './invocation-setup-fixtures';
import { summarizeInvocationBaseline, validateInvocationCapture } from './invocation-setup-measurement';

export function traceQuality(spans: any[]) {
  const ids = (process: string) => {
    const counts = new Map<string, number>();
    for (const span of spans.filter(row => row.process === process)) counts.set(span.spanId, (counts.get(span.spanId) ?? 0) + 1);
    return { raw: [...counts.values()].reduce((sum, count) => sum + count, 0),
      unique: counts.size, repeatedIds: [...counts.values()].filter(count => count > 1).length };
  };
  const host = ids('host'), worker = ids('worker');
  const scoped = spans.filter(span => span.process === 'worker' && span.invocationId !== undefined);
  const keys = scoped.map(span => `${span.invocationId}:${span.name}`);
  return { host, worker, hostQuantitativeAttributionUsable: host.repeatedIds === 0,
    workerInvocationScopedKeysUnique: new Set(keys).size === keys.length,
    globalSpanIdsUnique: new Set(spans.map(span => span.spanId)).size === spans.length,
    note: 'Raw records retained without deduplication. Worker span IDs share the process PID and restart in each fresh worker; unscoped bootstrap records cannot form a globally unique span tree.' };
}

if (import.meta.main) {
  const args = process.argv.slice(2); assert.equal(args.length, 3, 'Use TIMING_DIRECTORY TRACE_DIRECTORY PROBE_DIRECTORY');
  const [timing, trace, probe] = args.map(path => resolve(path));
  const read = async (path: string) => JSON.parse(await readFile(path, 'utf8'));
  const input = await read(join(timing!, 'inputs.json'));
  assert.equal(input.mode, 'timing'); assert.deepEqual(input.contract, contract);
  for (const [directory, mode] of [[trace, 'trace'], [probe, 'probe']]) {
    const other = await read(join(directory!, 'inputs.json'));
    assert.equal(other.mode, mode); assert.deepEqual(other.receipt, input.receipt); assert.deepEqual(other.contract, contract);
  }
  // Check retained artifacts and archived harness copies, even if HEAD later advances.
  for (const artifact of input.receipt.artifacts) assert.equal(sha256(await readFile(artifact.path)), artifact.sha256);
  const receiptDirectory = join(input.receipt.artifacts[0].path, '..');
  for (const file of input.receipt.inputs.harness) assert.equal(sha256(await readFile(join(receiptDirectory, 'harness', file.path))), file.sha256);
  const reports = [];
  for (let index = 0; index < contract.processesPerArtifact; index++) reports.push(await read(join(timing!, `process${index}/capture.json`)));
  const baseline = summarizeInvocationBaseline(reports);
  validateInvocationCapture(await read(join(trace!, 'process0/capture.json')), 'trace');
  const quality = [];
  for (const cohort of cohorts) {
    const spans = (await readFile(join(trace!, `process0/${cohort}.ndjson`), 'utf8')).trim().split('\n').map(line => JSON.parse(line));
    quality.push({ cohort, ...traceQuality(spans) });
  }
  const feasibility = await read(join(probe!, 'process0/probe.json'));
  assert.equal(feasibility.profile, 'woml.invocation-setup-feasibility/v1');
  assert.ok(feasibility.results.every((row: any) => row.compiledFunctionTransfer === 'DataCloneError'));
  const evidence = { profile: 'woml.invocation-setup-diagnosis/v1', baseline, quality, feasibility,
    analysisSha256: sha256(await readFile(import.meta.path)), rawDirectories: { timing, trace, probe },
    decision: 'No-go for production compilation/used-worker pooling or source-map preparation caching on the current evidence; retain fresh workers. No supported worthwhile reusable preparation has been established.',
    releaseAccepted: false, limitation: 'Baseline plus separate limited diagnostic captures, not a baseline/candidate speedup, native settlement test, complete bootstrap attribution or platform release pass.' };
  const output = join(timing!, 'diagnosis.json');
  assert.equal(await Bun.file(output).exists(), false, 'Never overwrite an earlier diagnosis');
  await Bun.write(output, JSON.stringify(evidence, null, 2));
  console.log(`ISR diagnosis: ${output}`);
}
