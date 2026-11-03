// ISR2 code-layout prototype only. No generated code is written into src/.
import assert from 'node:assert/strict';
import { resolve } from 'node:path';
import contract from '../../docs/invocation-facade-prototype.v1.json';
import { sha256 } from './invocation-setup-fixtures';

export const originalWorkerPath = resolve(import.meta.dir, '../src/script-host/worker.ts');

export function prototypeFacade(source: string) {
  // Not a general source rewriter. Exact identity makes the manually reviewed
  // four-capture argument fail closed on any source drift.
  assert.equal(sha256(source), contract.originalWorkerSourceSha256, 'Re-review source and closure captures before changing this prototype');
  const owner = source.indexOf('function deeplyReadonlyServiceFacade(');
  const start = source.indexOf('async (', source.indexOf('const invokeCapability =', owner));
  const bodyStart = source.indexOf(' => {', start) + 4;
  const end = source.indexOf('\n  };\n  return new Proxy(Object.freeze({}), {', bodyStart);
  assert.ok(owner >= 0 && start > owner && bodyStart > start && end > bodyStart);
  const signature = source.slice(start + 'async '.length, bodyStart - 4);
  const body = source.slice(bodyStart, end) + '\n  }';
  assert.ok(signature.endsWith(': Promise<JsonValue>'));
  const parameters = signature.slice(1, signature.lastIndexOf(')'));
  const helper = `async function dispatchCapabilityForAttempt(\nrequest: ScriptWorkerRequest,\nattempt: ScriptAttempt,\nresolvedSecretValues: readonly string[],\nexecutionDeadline: number,\n${parameters}\n): Promise<JsonValue> ${body}\n\n`;
  // Synchronous PRIVATE forwarding closure returns the original async Promise.
  // Eager Object.values/Map construction still precedes any user/module code.
  const forward = `${signature} => dispatchCapabilityForAttempt(${contract.captures.join(', ')}, ${contract.privateParameters.join(', ')})`;
  const transformed = source.slice(0, owner) + helper + source.slice(owner, start) + forward + source.slice(end + '\n  }'.length);
  new Bun.Transpiler({ loader: 'ts', target: 'bun' }).transformSync(transformed);
  return { source: transformed, originalBody: body, captures: contract.captures,
    rationale: 'No retained preparation object: one unchanged eager request-bound closure per fresh worker. Only private code layout changes; dispatch body is verbatim.' };
}
