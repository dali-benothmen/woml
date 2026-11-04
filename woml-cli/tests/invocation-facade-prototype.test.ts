import { expect, test } from 'bun:test';
import { readFile } from 'node:fs/promises';
import contract from '../../docs/invocation-facade-prototype.v1.json';
import acceptance from '../../docs/invocation-setup-acceptance.v1.json';
import { originalWorkerPath, prototypeFacade } from '../scripts/invocation-facade-transform';
import { assertRebuiltArtifact, summarizeFacade } from '../scripts/invocation-facade-prototype';
import { cohorts, fixture } from '../scripts/invocation-setup-fixtures';

// Git checkouts may use CRLF on Windows. Contract tests use repository-text LF;
// the actual build/capture guard still requires exact measured source bytes.
const sourceForTest = async () => (await readFile(originalWorkerPath, 'utf8')).replaceAll('\r\n', '\n');

function report(milliseconds: number, regression?: string) {
  return { profile: 'woml.invocation-setup-capture/v1', mode: 'timing', releaseAccepted: false,
    rows: cohorts.map(cohort => {
      const spec = fixture(cohort), requestRows = [{ batch: 'first', index: 0, wallMs: 1, hostDurationMs: 1 }];
      const batches = Array.from({ length: acceptance.warmupBatches + acceptance.measuredBatches }, (_, sample) => {
        for (let index = 0; index < spec.count; index++) requestRows.push({ batch: `sample${sample}`, index, wallMs: 1, hostDurationMs: 1 });
        return { sample, warmup: sample === 0, invocations: spec.count, wallMs: sample === 0 ? 9999 : cohort === regression ? 120 : milliseconds };
      });
      return { cohort, fixtureSha256: spec.fixtureSha256, concurrency: spec.concurrency, readyMs: 1, registrationMs: 1, firstInvocationMs: 1,
        teardownMs: 1, lifetimeMs: 100, batches, requestRows, completed: requestRows.length, allocationBytes: null, scopedPeakLiveBytes: null, cpuMs: null };
    }) };
}

test('private extraction preserves the exact dispatch body and all other worker bytes', async () => {
  const source = await sourceForTest(), result = prototypeFacade(source);
  expect(result.captures).toEqual(['request', 'attempt', 'resolvedSecretValues', 'executionDeadline']);
  expect(result.source.split(result.originalBody)).toHaveLength(2);
  expect(source.split(result.originalBody)).toHaveLength(2);
  const owner = source.indexOf('function deeplyReadonlyServiceFacade(');
  const start = source.indexOf('async (', source.indexOf('const invokeCapability =', owner));
  const end = source.indexOf('\n  };\n  return new Proxy(Object.freeze({}), {', start) + '\n  }'.length;
  const helperEnd = result.source.indexOf('function deeplyReadonlyServiceFacade(');
  const candidateStart = result.source.indexOf('const invokeCapability =', helperEnd) + 'const invokeCapability = '.length;
  const candidateEnd = result.source.indexOf(';\n  return new Proxy(Object.freeze({}), {', candidateStart);
  expect(result.source.slice(0, owner)).toBe(source.slice(0, owner));
  expect(result.source.slice(helperEnd, candidateStart)).toBe(source.slice(owner, start));
  expect(result.source.slice(candidateEnd)).toBe(source.slice(end));
  expect(result.source.slice(candidateStart, candidateEnd)).not.toStartWith('async');
  expect(result.source.indexOf('Object.values(request.bindings.secrets)', helperEnd)).toBeLessThan(candidateStart);
  expect(result.source.indexOf('const capabilityCache = new Map', helperEnd)).toBeLessThan(candidateStart);
  expect(contract.releaseAccepted).toBe(false);
});
test('source drift, extra captures, or a second transformation fail closed', async () => {
  const source = await sourceForTest();
  for (const changed of [source + '\n', source.replaceAll('\n', '\r\n'), source.replace('const managedHttp =', 'void materializedPaths; const managedHttp ='), prototypeFacade(source).source]) {
    expect(() => prototypeFacade(changed)).toThrow('Re-review');
  }
});
test('relocated builds may differ only in debug metadata, never executable bytes or source-map contents', () => {
  const original = 'same();\n//# debugId=AA11\n', rebuilt = 'same();\n//# debugId=BB22\n';
  expect(() => assertRebuiltArtifact('host.js', rebuilt, original, '/tmp/a/b', '/tmp/a')).not.toThrow();
  expect(() => assertRebuiltArtifact('host.js', rebuilt.replace('same', 'other'), original, '/tmp/a/b', '/tmp/a')).toThrow();
  const before = { version: 3, debugId: 'AA11', sources: ['src/a.ts'], sourcesContent: ['same'], mappings: 'AAA' };
  const after = { ...before, debugId: 'BB22', sources: ['../src/a.ts'] };
  expect(() => assertRebuiltArtifact('host.js.map', JSON.stringify(after), JSON.stringify(before), '/tmp/a/b', '/tmp/a')).not.toThrow();
  expect(() => assertRebuiltArtifact('host.js.map', JSON.stringify({ ...after, mappings: 'BBB' }), JSON.stringify(before), '/tmp/a/b', '/tmp/a')).toThrow();
});
test('matched comparison uses the original target, rejects missing processes and never certifies integration', () => {
  const reference = Array.from({ length: 7 }, () => report(100));
  expect(summarizeFacade(reference, Array.from({ length: 7 }, () => report(95))).numericalPrimaryTargetMet).toBe(false);
  const faster = summarizeFacade(reference, Array.from({ length: 7 }, () => report(80, 'inline')));
  expect(faster.numericalPrimaryTargetMet).toBe(true); expect(faster.controlInvestigations).toEqual(['inline']);
  expect(faster.integrationAccepted).toBe(false); expect(faster.releaseAccepted).toBe(false); expect(faster.cpuMs).toBeNull();
  expect(() => summarizeFacade(reference.slice(1), reference)).toThrow();
});
