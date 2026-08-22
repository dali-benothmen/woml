import { describe, expect, test } from 'bun:test';

import {
  compileWoml,
  parseWoml,
  WomlCompileError,
  WomlParseError,
  WomlValidationError,
} from '../src';

function generatedWorkflow(index: number, concurrency: number): string {
  const loadId = `loadItems${index}`;
  const loopId = `processItems${index}`;
  const bodyId = `transformItem${index}`;
  const summaryId = `summarizeItems${index}`;
  return `<woml>
  <workflow id="generated-loop-${index}" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps>
      <step id="${loadId}"><script>return { items: context.payload.items ?? [] };</script></step>
      <for-each id="${loopId}" items="{{context.steps.${loadId}.items}}" concurrency="${concurrency}">
        <step id="${bodyId}"><script>return { value: context.item, index: context.iteration.index };</script></step>
        <result value="{{context.steps.${bodyId}}}" />
      </for-each>
      <step id="${summaryId}"><script>return context.steps.${loopId};</script></step>
    </steps>
  </workflow>
</woml>`;
}

describe('WOML for-each hardening', () => {
  test('keeps generated identities deterministic and collision-free across valid limits', () => {
    const publicIds = new Set<string>();
    const engineIds = new Set<string>();
    for (let index = 0; index < 128; index += 1) {
      const concurrency = (index % 64) + 1;
      const source = generatedWorkflow(index, concurrency);
      const first = compileWoml(parseWoml(source, { file: `generated-${index}.woml` }));
      const second = compileWoml(parseWoml(source, { file: `generated-${index}.woml` }));
      expect(second).toEqual(first);
      expect(first.schemaVersion).toBe(16);
      if (first.schemaVersion !== 16) throw new Error('Expected Model v16.');

      const loop = first.graph.forEach[0]!;
      expect(loop.forEachId).toBe(`processItems${index}`);
      expect(loop.openNodeId).toBe(`__woml_for_each__processItems${index}__open`);
      expect(loop.resultNodeId).toBe(loop.forEachId);
      expect(loop.concurrency).toBe(concurrency);
      expect(loop.body.nodes.map(node => node.id)).toEqual([`transformItem${index}`]);
      expect(first.graph.nodes.map(node => node.id)).not.toContain(`transformItem${index}`);

      expect(publicIds.has(loop.forEachId)).toBe(false);
      expect(engineIds.has(loop.openNodeId)).toBe(false);
      publicIds.add(loop.forEachId);
      engineIds.add(loop.openNodeId);
    }
  });

  test('never accepts fuzzed reference escape spellings as loop item sources', () => {
    const invalidReferences = [
      '{{context.item.children}}',
      '{{context.iteration.index}}',
      '{{context.steps.missing.items}}',
      '{{context.steps.processItems0.results}}',
      '{{context.trigger.items}}',
      'prefix {{context.payload.items}}',
      '{{context.payload.items}} suffix',
      '{{context.payload["items"]}}',
      '{{secrets.ITEMS}}',
    ];
    for (const [index, reference] of invalidReferences.entries()) {
      const source = generatedWorkflow(index, 1).replace(
        `{{context.steps.loadItems${index}.items}}`,
        reference
      );
      try {
        compileWoml(parseWoml(source, { file: `invalid-${index}.woml` }));
        throw new Error(`Expected ${reference} to be rejected.`);
      } catch (error) {
        expect(
          error instanceof WomlValidationError ||
            error instanceof WomlCompileError ||
            error instanceof WomlParseError
        ).toBe(true);
        if (!(
          error instanceof WomlValidationError ||
          error instanceof WomlCompileError ||
          error instanceof WomlParseError
        ))
          throw error;
        expect([
          'WOML_FOR_EACH_ITEMS_INVALID',
          'WOML_FOR_EACH_ITEMS_NOT_VISIBLE',
          'WOML_UNKNOWN_REFERENCE',
          'WOML_INVALID_ATTRIBUTE',
          'WOML_SECRET_SINK_UNSUPPORTED',
        ]).toContain(error.diagnostic.code);
        expect(error.diagnostic.location.start.line).toBeGreaterThan(0);
        expect(error.diagnostic.location.start.column).toBeGreaterThan(0);
      }
    }
  });
});
