import assert from 'node:assert/strict';

import { compileWoml, parseWoml, type CompiledWorkflowDefinition } from '@woml/compiler';

function normalizeLineEndings(source: string): string {
  return source.replace(/\r\n/gu, '\n');
}

// This is an explicit migration of one reviewed example, not a general source
// equivalence rule. Keep the historical source/model/event fixtures unchanged.
const reviewedEdits = [
  ['version="0.1"', 'version="1.0.0"'],
  ['context.trigger.name', 'context.payload.name'],
] as const;

export function verifyRetryExampleContract(
  exampleSource: string,
  fixtureSource: string,
  frozenModel: unknown,
): CompiledWorkflowDefinition {
  const historicalSource = normalizeLineEndings(fixtureSource);
  const historicalModel = compileWoml(parseWoml(historicalSource, { file: 'retry.woml' }));
  assert.deepStrictEqual(
    historicalModel,
    frozenModel,
    'RI7 verification failed: the historical retry source must compile to its frozen Model v6 fixture.',
  );

  let expectedSource = historicalSource;
  for (const [before, after] of reviewedEdits) {
    assert.strictEqual(
      expectedSource.split(before).length - 1,
      1,
      `RI7 verification failed: the reviewed retry migration requires exactly one ${before}.`,
    );
    expectedSource = expectedSource.replace(before, after);
  }
  assert.strictEqual(
    normalizeLineEndings(exampleSource),
    expectedSource,
    'RI7 verification failed: the public retry example may differ only by workflow version 0.1 -> 1.0.0 and context.trigger.name -> context.payload.name (plus CRLF/LF line endings).',
  );

  // Also validate the author-facing source. Exact migrated-source equality
  // preserves every retry attribute, script, trigger and graph declaration.
  return compileWoml(parseWoml(normalizeLineEndings(exampleSource), { file: 'retryWorkflow.woml' }));
}
