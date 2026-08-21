import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

import {
  compileWoml,
  isWomlElement,
  parseWoml,
  validateWoml,
  WomlCompileError,
  WomlValidationError,
  type WomlSourceElement,
} from '../src';

function workflow(body: string): string {
  return `<woml>
  <workflow id="for-each-test" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps>${body}</steps>
  </workflow>
</woml>`;
}

function diagnostic(source: string): WomlValidationError | WomlCompileError {
  try {
    validateWoml(parseWoml(source, { file: 'for-each-test.woml' }));
  } catch (error) {
    if (
      error instanceof WomlValidationError ||
      error instanceof WomlCompileError
    ) {
      return error;
    }
    throw error;
  }
  throw new Error('Expected WOML validation to fail.');
}

function validLoop(
  attributes = 'id="iterate" items="{{context.steps.load.items}}"',
  children = '<step id="work"><script>return context.item;</script></step>'
): string {
  return workflow(`
    <step id="load"><script>return { items: [1, 2] };</script></step>
    <for-each ${attributes}>${children}</for-each>
    <step id="done"><script>return true;</script></step>
  `);
}

describe('WOML for-each authoring', () => {
  test('preserves the reviewed tag, attributes, body order, result, and source spans', () => {
    const source = readFileSync(
      new URL('./fixtures/for-each/authoring.reviewed.woml', import.meta.url),
      'utf8'
    );
    const document = parseWoml(source, { file: 'authoring.reviewed.woml' });
    const workflowElement = document.root.children.find(
      (child): child is WomlSourceElement =>
        isWomlElement(child) && child.name === 'workflow'
    )!;
    const steps = workflowElement.children.find(
      (child): child is WomlSourceElement =>
        isWomlElement(child) && child.name === 'steps'
    )!;
    const forEach = steps.children.find(
      (child): child is WomlSourceElement =>
        isWomlElement(child) && child.name === 'for-each'
    )!;
    const children = forEach.children.filter(isWomlElement);

    expect(forEach.attributes).toMatchObject({
      id: { value: 'organize' },
      name: { value: 'Organize files' },
      description: { value: 'Choose one destination for every file.' },
      items: { value: '{{context.steps.scan.files}}' },
      concurrency: { value: '2' },
    });
    expect(children.map(child => child.name)).toEqual(['switch', 'result']);
    expect(forEach.openTagSpan.start.offset).toBe(source.indexOf('<for-each'));
    expect(forEach.attributes.items.valueSpan.start.offset).toBe(
      source.indexOf('{{context.steps.scan.files}}')
    );
  });

  test('validates iteration references, ordered results, and the default concurrency profile', () => {
    const source = readFileSync(
      new URL('./fixtures/for-each/authoring.reviewed.woml', import.meta.url),
      'utf8'
    );
    expect(() =>
      validateWoml(parseWoml(source, { file: 'authoring.reviewed.woml' }))
    ).not.toThrow();
    expect(() => validateWoml(parseWoml(validLoop()))).not.toThrow();
  });

  test('requires id and items and rejects the retired source spelling', () => {
    expect(
      diagnostic(validLoop('items="{{context.steps.load.items}}"')).diagnostic
        .code
    ).toBe('WOML_FOR_EACH_ID_REQUIRED');
    expect(diagnostic(validLoop('id="iterate"')).diagnostic.code).toBe(
      'WOML_FOR_EACH_ITEMS_REQUIRED'
    );
    expect(
      diagnostic(
        validLoop('id="iterate" source="{{context.steps.load.items}}"')
      ).diagnostic.code
    ).toBe('WOML_FOR_EACH_ATTRIBUTE_UNKNOWN');
  });

  test('requires one exact upstream array reference', () => {
    for (const attributes of [
      'id="iterate" items="context.steps.load.items"',
      'id="iterate" items="{{context.item.children}}"',
      'id="iterate" items="{{context.trigger.items}}"',
      'id="iterate" items="prefix {{context.steps.load.items}}"',
    ]) {
      expect(diagnostic(validLoop(attributes)).diagnostic.code).toBe(
        'WOML_FOR_EACH_ITEMS_INVALID'
      );
    }

    const later = workflow(`
      <for-each id="iterate" items="{{context.steps.load.items}}">
        <step id="work"><script>return context.item;</script></step>
      </for-each>
      <step id="load"><script>return { items: [] };</script></step>
    `);
    expect(diagnostic(later).diagnostic.code).toBe(
      'WOML_FOR_EACH_ITEMS_NOT_VISIBLE'
    );
  });

  test('validates concurrency, body cardinality, and final result placement', () => {
    for (const value of ['0', '1.5', '65', 'many']) {
      expect(
        diagnostic(
          validLoop(
            `id="iterate" items="{{context.steps.load.items}}" concurrency="${value}"`
          )
        ).diagnostic.code
      ).toBe('WOML_FOR_EACH_CONCURRENCY_INVALID');
    }
    expect(diagnostic(validLoop(undefined, '')).diagnostic.code).toBe(
      'WOML_FOR_EACH_EMPTY'
    );
    expect(
      diagnostic(
        validLoop(
          undefined,
          '<result value="{{context.item}}" /><step id="work"><script>return context.item;</script></step>'
        )
      ).diagnostic.code
    ).toBe('WOML_FOR_EACH_RESULT_ORDER');
    expect(
      diagnostic(
        validLoop(
          undefined,
          '<step id="work"><script>return context.item;</script></step><result />'
        )
      ).diagnostic.code
    ).toBe('WOML_FOR_EACH_RESULT_INVALID');
  });

  test('rejects nested loops, forks, and approvals in the first profile', () => {
    const nested = [
      '<for-each id="nested" items="{{context.steps.load.items}}"><step id="nestedWork"><script>return context.item;</script></step></for-each>',
      '<fork id="fanout"><branch id="one"><step id="forkWork"><script>return true;</script></step></branch></fork>',
      '<approval id="review"><when-approved /><when-rejected /></approval>',
    ];
    for (const child of nested) {
      expect(diagnostic(validLoop(undefined, child)).diagnostic.code).toBe(
        'WOML_FOR_EACH_NESTING_UNSUPPORTED'
      );
    }
  });

  test('keeps iteration-only references out of ordinary control flow', () => {
    const source = workflow(`
      <switch value="{{context.item.category}}">
        <case value="a"><step id="a"><script>return true;</script></step></case>
        <default><step id="b"><script>return false;</script></step></default>
      </switch>
      <step id="done"><script>return true;</script></step>
    `);
    expect(diagnostic(source).diagnostic.code).toBe(
      'WOML_ITERATION_REFERENCE_OUTSIDE_FOR_EACH'
    );
  });

  test('routes valid authoring only through the frozen Model v16 contract', () => {
    const document = parseWoml(validLoop(), { file: 'for-each-test.woml' });
    expect(() => validateWoml(document)).not.toThrow();
    const compiled = compileWoml(document);
    expect(compiled.schemaVersion).toBe(16);
    expect(compiled.graph).toHaveProperty('forEach');
  });
});
