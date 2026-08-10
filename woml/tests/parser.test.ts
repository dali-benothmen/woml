import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

import {
  isWomlElement,
  isWomlRawText,
  parseWoml,
  WomlParseError,
  type WomlSourceElement,
} from '../src';

function elementChildren(element: WomlSourceElement): WomlSourceElement[] {
  return element.children.filter(isWomlElement);
}

function workflowElement(root: WomlSourceElement): WomlSourceElement {
  const workflow = elementChildren(root).find(child => child.name === 'workflow');
  if (workflow === undefined) throw new Error('Missing fixture workflow.');
  return workflow;
}

function parseError(source: string): WomlParseError {
  try {
    parseWoml(source, { file: 'broken.woml' });
  } catch (error) {
    if (error instanceof WomlParseError) return error;
    throw error;
  }
  throw new Error('Expected WOML parsing to fail.');
}

describe('parseWoml', () => {
  test('parses the Phase 0 workflow into an ordered, source-aware tree', () => {
    const source = readFileSync(
      new URL('./fixtures/hello.woml', import.meta.url),
      'utf8'
    );
    const document = parseWoml(source, { file: 'hello.woml' });

    expect(document.root.name).toBe('woml');
    expect(document.root.span.start).toEqual({ line: 1, column: 1, offset: 0 });
    expect(document.root.span.end.offset).toBe(
      source.lastIndexOf('</woml>') + '</woml>'.length
    );
    expect(document.span.end.offset).toBe(source.length);

    const workflow = workflowElement(document.root);
    expect(workflow.attributes.id.value).toBe('hello');
    expect(workflow.attributes.name.value).toBe('Hello WOML');
    const [triggers, steps] = elementChildren(workflow);
    expect([triggers.name, steps.name]).toEqual(['triggers', 'steps']);
    expect(elementChildren(triggers)[0].attributes.id.value).toBe('start');

    const [stepA, stepB] = elementChildren(steps);
    expect(stepA.attributes).toMatchObject({
      id: { value: 'a' },
      name: { value: 'Choose greeting name' },
      description: { value: 'Use the trigger name or default to World' },
    });
    expect(stepB.attributes.id.value).toBe('b');

    const scriptA = elementChildren(stepA)[0];
    const scriptB = elementChildren(stepB)[0];
    const rawA = scriptA.children[0];
    const rawB = scriptB.children[0];
    expect(isWomlRawText(rawA)).toBe(true);
    expect(isWomlRawText(rawB)).toBe(true);
    if (!isWomlRawText(rawA) || !isWomlRawText(rawB)) return;

    expect(rawA.value).toContain(
      'const name = context.trigger.name ?? "World";'
    );
    expect(rawB.value).toContain('`Hello ${context.steps.a.x}`');
    expect(source.slice(rawA.span.start.offset, rawA.span.end.offset)).toBe(
      rawA.value
    );
    expect(source.slice(rawB.span.start.offset, rawB.span.end.offset)).toBe(
      rawB.value
    );
  });

  test('preserves XML-significant JavaScript byte-for-byte', () => {
    const rawScript = `
        const markup = \`<item>\${context.trigger.value}</item>\`;
        if (context.trigger.score < 0.8 && context.trigger.enabled) {
          return { markup, accepted: true };
        }
        return { markup, accepted: false };
      `;
    const source = `<workflow version="1.0.0" id="raw-script">
  <triggers><manual id="start" /></triggers>
  <steps><step id="a"><script>${rawScript}</script></step></steps>
</workflow>`;

    const document = parseWoml(source, { file: 'raw-script.woml' });
    const steps = elementChildren(document.root)[1];
    const step = elementChildren(steps)[0];
    const script = elementChildren(step)[0];
    const raw = script.children[0];

    expect(isWomlRawText(raw)).toBe(true);
    if (!isWomlRawText(raw)) return;
    expect(raw.value).toBe(rawScript);
    expect(source.slice(raw.span.start.offset, raw.span.end.offset)).toBe(
      rawScript
    );
  });

  test('preserves branch case order, reference values, and source spans', () => {
    const source = readFileSync(
      new URL('./fixtures/branch.woml', import.meta.url),
      'utf8'
    );
    const document = parseWoml(source, { file: 'branch.woml' });
    const [, steps] = elementChildren(workflowElement(document.root));
    const branch = elementChildren(steps)[1];
    const [when, otherwise] = elementChildren(branch);
    const whenResult = elementChildren(when).at(-1);

    expect(branch.name).toBe('branch');
    expect(branch.attributes.id.value).toBe('decision');
    expect([when.name, otherwise.name]).toEqual(['when', 'otherwise']);
    expect(when.attributes.test.value).toBe(
      '{{context.steps.checkContent.needsReview}}'
    );
    expect(whenResult?.name).toBe('result');
    expect(whenResult?.attributes.value.value).toBe(
      '{{context.steps.reviewContent}}'
    );
    expect(branch.openTagSpan.start.offset).toBe(source.indexOf('<branch'));
    expect(when.attributes.test.valueSpan.start.offset).toBe(
      source.indexOf('{{context.steps.checkContent.needsReview}}')
    );
  });

  test('preserves parallel child order, attributes, scripts, and source spans', () => {
    const source = readFileSync(
      new URL('./fixtures/parallel.woml', import.meta.url),
      'utf8'
    );
    const document = parseWoml(source, { file: 'parallel.woml' });
    const [, steps] = elementChildren(workflowElement(document.root));
    const parallel = elementChildren(steps)[1];
    const [weather, soil] = elementChildren(parallel);
    const weatherScript = elementChildren(weather)[0].children[0];

    expect(parallel.name).toBe('parallel');
    expect(parallel.attributes).toMatchObject({
      id: { value: 'fieldData' },
      name: { value: 'Load field data' },
      description: { value: 'Load independent readings' },
      concurrency: { value: '2' },
      'on-error': { value: 'wait-all' },
    });
    expect([weather.attributes.id.value, soil.attributes.id.value]).toEqual([
      'loadWeather',
      'loadSoil',
    ]);
    expect(isWomlRawText(weatherScript)).toBe(true);
    if (!isWomlRawText(weatherScript)) return;
    expect(weatherScript.value).toContain('context.steps.loadField.fieldId');
    expect(parallel.openTagSpan.start.offset).toBe(source.indexOf('<parallel'));
    expect(
      source.slice(
        weatherScript.span.start.offset,
        weatherScript.span.end.offset
      )
    ).toBe(weatherScript.value);
  });

  test('ignores markup-looking text inside script bodies', () => {
    const source = `<workflow>
  <script>
    const brokenMarkup = "<step><not-closed>";
    return { brokenMarkup };
  </script>
</workflow>`;

    const document = parseWoml(source, { file: 'script-markup.woml' });
    const script = elementChildren(document.root)[0];
    const raw = script.children[0];
    expect(isWomlRawText(raw) && raw.value).toContain('<step><not-closed>');
  });

  test('reports duplicate attributes at their original location', () => {
    const source = '<workflow id="first" id="second" />';
    const error = parseError(source);
    const repeatedAttributeOffset = source.lastIndexOf('id=');

    expect(error.diagnostic.code).toBe('WOML_DUPLICATE_ATTRIBUTE');
    expect(error.diagnostic.file).toBe('broken.woml');
    expect(error.diagnostic.location.start).toEqual({
      line: 1,
      column: repeatedAttributeOffset + 1,
      offset: repeatedAttributeOffset,
    });
  });

  test('rejects declarations before parsing the tree', () => {
    const source = '<?xml version="1.0"?>\n<workflow />';
    const error = parseError(source);

    expect(error.diagnostic.code).toBe('WOML_DECLARATION_NOT_ALLOWED');
    expect(error.diagnostic.location.start).toEqual({
      line: 1,
      column: 1,
      offset: 0,
    });
  });

  test('rejects a second root at the second root location', () => {
    const source = '<workflow />\n<workflow />';
    const error = parseError(source);
    const secondRootOffset = source.lastIndexOf('<workflow');

    expect(error.diagnostic.code).toBe('WOML_MULTIPLE_ROOTS');
    expect(error.diagnostic.location.start).toEqual({
      line: 2,
      column: 1,
      offset: secondRootOffset,
    });
  });

  test('reports an unclosed raw body at its opening script tag', () => {
    const source = `<workflow>
  <script>
    return { ok: true };
</workflow>`;
    const error = parseError(source);

    expect(error.diagnostic.code).toBe('WOML_UNCLOSED_RAW_BODY');
    expect(error.diagnostic.location.start).toEqual({
      line: 2,
      column: 3,
      offset: source.indexOf('<script>'),
    });
  });

  test('maps malformed markup before a script to the original source', () => {
    const source = `<workflow>
  <triggers>
  </trigger>
  <script>return { ok: true };</script>
</workflow>`;
    const error = parseError(source);
    const badClosingOffset = source.indexOf('</trigger>');

    expect(error.diagnostic.code).toBe('WOML_MALFORMED_MARKUP');
    expect(error.diagnostic.location.start).toEqual({
      line: 3,
      column: 3,
      offset: badClosingOffset,
    });
  });

  test('maps malformed markup after a script to the original source', () => {
    const source = `<workflow>
  <script>
    return context.trigger.value < 10 && context.trigger.enabled;
  </script>
  <steps>
  </stepz>
</workflow>`;
    const error = parseError(source);
    const badClosingOffset = source.indexOf('</stepz>');

    expect(error.diagnostic.code).toBe('WOML_MALFORMED_MARKUP');
    expect(error.diagnostic.location.start).toEqual({
      line: 6,
      column: 3,
      offset: badClosingOffset,
    });
  });

  test('rejects non-whitespace text after the root', () => {
    const source = '<workflow />\nnot markup';
    const error = parseError(source);

    expect(error.diagnostic.code).toBe('WOML_TEXT_OUTSIDE_ROOT');
    expect(error.diagnostic.location.start).toEqual({
      line: 2,
      column: 1,
      offset: source.indexOf('not markup'),
    });
  });

  test('allows comments around the single root', () => {
    const source = '<!-- before -->\n<workflow />\n<!-- after -->';
    const document = parseWoml(source, { file: 'comments.woml' });

    expect(document.root.name).toBe('workflow');
    expect(document.root.span.start.offset).toBe(source.indexOf('<workflow'));
  });

  test('explains the literal raw-body terminator restriction at the original location', () => {
    const source = `<workflow>
  <script>
    const marker = "</script>";
    return marker;
  </script>
</workflow>`;
    const error = parseError(source);

    expect(error.diagnostic.code).toBe('WOML_MALFORMED_MARKUP');
    expect(error.diagnostic.location.start.line).toBe(5);
    expect(error.diagnostic.hint).toContain('first literal </script>');
  });
});
