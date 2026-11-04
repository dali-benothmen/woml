import { createHash } from 'node:crypto';
import contract from '../../docs/invocation-setup-acceptance.v1.json';
import type { ExecuteMessageV12, RegisterModuleMessage } from '../src/script-host/types';

export const sha256 = (value: string | Uint8Array) => createHash('sha256').update(value).digest('hex');
export const cohorts = ['mixed', ...contract.controls] as const;
export type Cohort = typeof cohorts[number];

export function fixture(cohort: Cohort) {
  const count = cohort === 'mixed' ? contract.primary.invocationsPerBatch : contract.controlInvocationsPerBatch;
  const concurrency = cohort === 'concurrency-4' ? 4 : cohort === 'concurrency-2' ? 2 : 1;
  const moduleCount = cohort === 'inline' ? 0 : cohort === 'unique' ? count : 1;
  const artifacts = Array.from({ length: moduleCount }, (_, index) => {
    const bundle = `let calls = 0; export function value() { return ++calls; }\n/*${cohort === 'large-module' ? 'b'.repeat(contract.largeModulePaddingBytes) : ''}fixture-${index}*/`;
    const sourceMap = JSON.stringify({ version: 3, sources: ['fixture.ts'], names: [], mappings: '',
      sourcesContent: [cohort === 'large-module' ? 's'.repeat(contract.largeSourceMapPaddingBytes) : 'synthetic fixture'] });
    return { protocol: 'woml.script-host', protocolVersion: 12, messageType: 'register_module',
      bundle, bundleDigest: `sha256:${sha256(bundle)}`, sourceMap, sourceMapDigest: `sha256:${sha256(sourceMap)}`,
    } satisfies RegisterModuleMessage;
  });
  function request(index: number, batch: string): ExecuteMessageV12 {
    const usesModule = moduleCount > 0 && (cohort !== 'mixed' || index % 2 === 1);
    const artifact = artifacts[cohort === 'unique' ? index : 0];
    return { protocol: 'woml.script-host', protocolVersion: 12, messageType: 'execute',
      invocationId: `inv_isr_${cohort}_${batch}_${index}`, runId: `run_isr_${batch}`, nodeId: `node${index}`,
      handler: 'runtime.script', mode: 'step', timeoutMs: contract.invocationTimeoutMs,
      attempt: { number: 1, maxAttempts: 1, idempotencyKey: `sha256:${'1'.repeat(64)}` },
      context: { trigger: { padding: cohort === 'context' ? 'c'.repeat(contract.contextPaddingBytes) : '' }, steps: {} },
      source: `if (globalThis.__isr_seen !== undefined) throw new Error('worker state leaked'); globalThis.__isr_seen = true; return ${usesModule ? 'services.utility.value()' : '1'};`,
      bindings: { bindingVersion: 1, servicesVersion: 1, secrets: {} },
      modules: usesModule ? [{ name: 'utility', bundleDigest: artifact!.bundleDigest, exports: ['value'] }] : [],
    };
  }
  // IDs differ by batch; all workload bytes and expected outputs stay fixed.
  const fixtureSha256 = sha256(JSON.stringify({ cohort, count, concurrency, artifacts,
    requests: Array.from({ length: count }, (_, index) => request(index, 'fingerprint')) }));
  return { cohort, count, concurrency, artifacts, request, fixtureSha256 };
}
