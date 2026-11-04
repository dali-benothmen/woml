// Component diagnosis in a fresh diagnostic worker. NOT the production loader,
// isolation proof, native measurement, or an acceptance timing sample.
import assert from 'node:assert/strict';
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;

function timed(operation: () => unknown, repetitions: number) {
  const before = performance.now();
  for (let index = 0; index < repetitions; index++) operation();
  return (performance.now() - before) / repetitions;
}
self.onmessage = async (event: MessageEvent<{ bundle: string; sourceMap: string }>) => {
  const { bundle, sourceMap } = event.data;
  const rows = [];
  for (let sample = 0; sample < 4; sample++) {
    let prepared = '';
    const digestMs = timed(() => {
      new Bun.CryptoHasher('sha256').update(bundle).digest('hex');
      new Bun.CryptoHasher('sha256').update(sourceMap).digest('hex');
    }, 100);
    const embedMs = timed(() => {
      // Same deterministic byte transformation as executableModuleBundle.
      const sourceMapUrl = Buffer.from(sourceMap, 'utf8').toString('base64');
      prepared = `${bundle.replace(/\n?\/\/# sourceMappingURL=.*$/m, '')}\n//# sourceMappingURL=data:application/json;base64,${sourceMapUrl}`;
    }, 100);
    assert.ok(prepared.endsWith(Buffer.from(sourceMap, 'utf8').toString('base64')));
    const compileMs = timed(() => { new AsyncFunction('context', 'return 1;'); }, 100);
    const before = performance.now();
    for (let index = 0; index < 10; index++) {
      const url = URL.createObjectURL(new Blob([prepared], { type: 'text/javascript' }));
      try { const namespace = await import(url); assert.equal(namespace.value(), 1); }
      finally { URL.revokeObjectURL(url); }
    }
    rows.push({ sample, warmup: sample === 0, digestMs, embedMs, compileMs, importMs: (performance.now() - before) / 10 });
  }
  self.postMessage({ rows });
};
