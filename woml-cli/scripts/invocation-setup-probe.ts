// Separate feasibility/component diagnosis. Never part of baseline timing.
import assert from 'node:assert/strict';
import { join } from 'node:path';
import { fixture } from './invocation-setup-fixtures';

const directory = process.argv[2]; assert.ok(directory);
const results = [];
for (const cohort of ['module', 'large-module']) {
  const spec = fixture(cohort), artifact = spec.artifacts[0]!;
  const worker = new Worker(new URL('./invocation-setup-probe-worker.ts', import.meta.url).href, { type: 'module', smol: true, env: {} });
  let transferFailure: string | null = null;
  try {
    const compiled = Object.getPrototypeOf(async function () {}).constructor('return 1;');
    worker.postMessage({ compiled });
  } catch (error) { transferFailure = error instanceof Error ? error.name : String(error); }
  assert.equal(transferFailure, 'DataCloneError', 'Do not infer compiled-function portability from a string cache');
  const result = await new Promise<any>((resolve, reject) => {
    const watchdog = setTimeout(() => { worker.terminate(); reject(new Error('ISR feasibility probe watchdog')); }, 30_000);
    worker.onmessage = event => { clearTimeout(watchdog); resolve(event.data); };
    worker.onerror = error => { clearTimeout(watchdog); worker.terminate(); reject(error); };
    worker.postMessage({ bundle: artifact.bundle, sourceMap: artifact.sourceMap });
  }).finally(() => worker.terminate());
  assert.equal(result.rows.length, 4);
  results.push({ cohort, fixtureSha256: spec.fixtureSha256, bundleBytes: Buffer.byteLength(artifact.bundle),
    mapBytes: Buffer.byteLength(artifact.sourceMap), encodedMapBytes: Buffer.byteLength(Buffer.from(artifact.sourceMap).toString('base64')),
    compiledFunctionTransfer: transferFailure, ...result });
}
await Bun.write(join(directory, 'probe.json'), JSON.stringify({ profile: 'woml.invocation-setup-feasibility/v1', results, releaseAccepted: false,
  limitation: 'Same Bun version, fresh diagnostic workers, copied byte transformation and import/compile probes. Not production-loader timing, not proof that every possible runtime caching API is unavailable.' }, null, 2));
