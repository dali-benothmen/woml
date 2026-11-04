// Real raw worker control versus an explicitly perturbed diagnostic wrapper.
import assert from 'node:assert/strict';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import contract from '../../docs/invocation-bootstrap-diagnostic.v1.json';
import { fixture } from './invocation-setup-fixtures';
import { resourceRecord } from './validated-state-baseline';

export function bootstrapOrder(sample: number, processIndex: number) {
  const rotated = contract.cases.map((_, index) => contract.cases[(index + sample + processIndex) % contract.cases.length]!);
  return processIndex % 2 ? rotated.reverse() : rotated;
}
export function bootstrapFixture(name: string) {
  return fixture(name === 'wrapped-context' ? 'context' : name === 'wrapped-large' ? 'large-module'
    : name.endsWith('inline') || name === 'wrapped-empty' ? 'inline' : 'module');
}
async function observe(entry: string, wrapper: string, name: string, sample: number) {
  const spec = bootstrapFixture(name), observerId = crypto.randomUUID(), envelope = spec.request(0, observerId);
  const request = { ...envelope, modules: envelope.modules.map(binding => {
    const artifact = spec.artifacts.find(item => item.bundleDigest === binding.bundleDigest)!;
    return { ...binding, bundle: artifact.bundle, sourceMap: artifact.sourceMap, sourceMapDigest: artifact.sourceMapDigest };
  }) };
  const raw = name.startsWith('raw-');
  const began = performance.now();
  const worker = new Worker(pathToFileURL(raw ? entry : wrapper).href, { type: 'module', smol: true, ref: true, env: {} });
  const constructorMs = performance.now() - began;
  let enteredMs: number | null = null, readyMs: number | null = null, importMs: number | null = null, realmId: string | null = null;
  let requestStarted = 0, stage = raw ? 'executing' : 'entered';
  try {
    return await new Promise<any>((resolve, reject) => {
      const watchdog = setTimeout(() => { worker.terminate(); reject(new Error(`Bootstrap watchdog: ${name}`)); }, contract.watchdogMs);
      worker.onerror = error => { clearTimeout(watchdog); reject(new Error(`Bootstrap worker failed: ${name}: ${error.message}`)); };
      worker.onmessage = event => {
        try {
          const message = event.data;
          if (!raw) {
            if (realmId !== null) assert.equal(message.realmId, realmId);
            if (message.kind === 'entered') {
              assert.equal(stage, 'entered'); assert.match(message.realmId, /^[a-f0-9-]{36}$/);
              realmId = message.realmId; enteredMs = performance.now() - began; stage = 'loading';
              worker.postMessage({ kind: 'configure', observerId, entry: pathToFileURL(entry).href, empty: name === 'wrapped-empty' }); return;
            }
            assert.equal(message.observerId, observerId);
            if (message.kind === 'ready') {
              assert.equal(stage, 'loading'); importMs = message.importMs; readyMs = performance.now() - began;
              stage = 'executing'; requestStarted = performance.now(); worker.postMessage({ kind: 'execute', observerId, request }); return;
            }
            assert.equal(message.kind, 'completed'); assert.equal(message.importMs, importMs);
          } else assert.equal(message.messageType, 'completed');
          assert.equal(stage, 'executing'); stage = 'completed';
          assert.deepEqual(message.response, { ok: true, result: 1 });
          const requestRoundTripMs = performance.now() - requestStarted;
          const journeyMs = performance.now() - began;
          clearTimeout(watchdog);
          resolve({ name, sample, warmup: sample < contract.warmupSamples, observerId, realmId, fixtureSha256: spec.fixtureSha256,
            constructorMs, enteredMs, readyMs, importMs, requestRoundTripMs, journeyMs,
            executionMs: raw ? null : message.executionMs, executedProductionCode: name !== 'wrapped-empty' });
        } catch (error) { clearTimeout(watchdog); reject(error); }
      };
      if (raw) { requestStarted = performance.now(); worker.postMessage({ messageType: 'execute', request }); }
    });
  } finally { worker.terminate(); }
}

if (import.meta.main) {
  const [entry, wrapper, directory, processIndex] = process.argv.slice(2);
  assert.ok(entry && wrapper && directory && /^\d+$/.test(processIndex!));
  const rows = [];
  for (let sample = 0; sample < contract.warmupSamples + contract.measuredSamples; sample++) {
    for (const name of bootstrapOrder(sample, Number(processIndex))) {
      const before = performance.now();
      const workers = await Promise.all(Array.from({ length: name === 'wrapped-concurrent' ? contract.concurrentWorkers : 1 },
        () => observe(entry, wrapper, name, sample)));
      rows.push({ name, sample, warmup: sample < contract.warmupSamples, batchMs: performance.now() - before, workers });
      await Bun.write(join(directory, 'capture.json'), JSON.stringify({ profile: 'woml.invocation-bootstrap-capture/v1',
        processIndex: Number(processIndex), rows, releaseAccepted: false, cpuMs: null, allocatedBytes: null, peakLiveBytes: null,
        processUsage: resourceRecord(undefined) }, null, 2));
    }
  }
}
