import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

import { compileWoml, parseWoml } from '../src';

function fixture(name: string): string {
  return readFileSync(
    new URL(`./fixtures/for-each/${name}`, import.meta.url),
    'utf8'
  );
}

describe('WOML for-each Model v16 lowering', () => {
  test('deep-equals the reviewed Model v16 fixture', () => {
    const source = fixture('model.reviewed.woml');
    const compiled = compileWoml(
      parseWoml(source, { file: 'model.reviewed.woml' })
    );
    expect(compiled).toEqual(JSON.parse(fixture('model.v16.reviewed.json')));
  });

  test('keeps the body as an immutable local DAG and exposes only the loop result outside', () => {
    const compiled = compileWoml(
      parseWoml(fixture('model.reviewed.woml'), {
        file: 'model.reviewed.woml',
      })
    );
    if (compiled.schemaVersion !== 16) {
      throw new Error('Expected Model v16.');
    }
    const loop = compiled.graph.forEach[0];
    expect(loop.outerStepIds).toEqual(['load']);
    expect(loop.body.entryNodeIds).toEqual(['normalize']);
    expect(loop.body.terminalNodeId).toBe('normalize');
    expect(loop.body.contextVisibility).toEqual([
      { nodeId: 'normalize', stepIds: [] },
    ]);
    expect(compiled.graph.contextVisibility).toEqual([
      { nodeId: 'load', stepIds: [] },
      { nodeId: 'summary', stepIds: ['load', 'organize'] },
    ]);
    expect(compiled.graph.nodes.map(node => node.id)).not.toContain('normalize');
  });

  test('lowers switch control flow inside the reviewed body template deterministically', () => {
    const source = fixture('authoring.reviewed.woml');
    const first = compileWoml(
      parseWoml(source, { file: 'authoring.reviewed.woml' })
    );
    const second = compileWoml(
      parseWoml(source, { file: 'authoring.reviewed.woml' })
    );
    expect(second).toEqual(first);
    if (first.schemaVersion !== 16) throw new Error('Expected Model v16.');
    const loop = first.graph.forEach[0];
    expect(loop.body.choices).toHaveLength(1);
    expect(loop.body.choices[0].stringSelector).toEqual({
      kind: 'contextReference',
      path: ['item', 'category'],
    });
    expect(loop.body.terminalNodeId).toBe('destination');
    expect(loop.result).toEqual({
      kind: 'contextReference',
      path: ['steps', 'destination'],
    });
  });
});
