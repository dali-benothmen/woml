import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

import {
  compileWoml,
  inspectCompiledWorkflowGraph,
  parseWoml,
  WomlValidationError,
  type CompiledWorkflowGraph,
} from '../src';

function compile(source: string) {
  return compileWoml(parseWoml(source, { file: 'workflow.woml' }));
}

function validationError(source: string): WomlValidationError {
  try {
    compile(source);
  } catch (error) {
    if (error instanceof WomlValidationError) return error;
    throw error;
  }
  throw new Error('Expected WOML validation to fail.');
}

function validWorkflow(stepMarkup = '<step id="a"><script>return { ok: true };</script></step>') {
  return `<workflow woml-version="0.1" id="test-workflow">
  <triggers><manual id="start" /></triggers>
  <steps>${stepMarkup}</steps>
</workflow>`;
}

describe('compileWoml', () => {
  test('deep-equals the reviewed Phase 0 compiled fixture', () => {
    const source = readFileSync(new URL('./fixtures/hello.woml', import.meta.url), 'utf8');
    const expected = JSON.parse(
      readFileSync(
        new URL('./fixtures/hello.compiled.v1.json', import.meta.url),
        'utf8',
      ),
    );

    const compiled = compileWoml(parseWoml(source, { file: 'hello.woml' }));

    expect(compiled).toEqual(expected);
    expect(
      inspectCompiledWorkflowGraph(compiled.graph, {
        requireSingleTerminal: true,
      }),
    ).toEqual([]);
  });

  test('lowers one script step to one entry and one terminal node', () => {
    const compiled = compile(validWorkflow());

    expect(compiled).toMatchObject({
      schemaVersion: 1,
      workflowId: 'test-workflow',
      triggers: [
        {
          id: 'start',
          handler: 'trigger.manual',
          config: { kind: 'object', fields: {} },
        },
      ],
      graph: {
        entryNodeIds: ['a'],
        nodes: [
          {
            id: 'a',
            handler: 'runtime.script',
            inputs: {
              kind: 'object',
              fields: {
                source: {
                  kind: 'literal',
                  value: 'return { ok: true };',
                },
              },
            },
          },
        ],
        edges: [],
      },
    });
  });

  test('turns sequential document order into explicit DAG edges', () => {
    const compiled = compile(
      validWorkflow(`
    <step id="first"><script>return 1;</script></step>
    <step id="second"><script>return 2;</script></step>
    <step id="third"><script>return 3;</script></step>`),
    );

    expect(compiled.graph.entryNodeIds).toEqual(['first']);
    expect(compiled.graph.nodes.map((node) => node.id)).toEqual([
      'first',
      'second',
      'third',
    ]);
    expect(compiled.graph.edges).toEqual([
      {
        id: 'first-to-second',
        from: 'first',
        to: 'second',
        condition: { kind: 'always' },
      },
      {
        id: 'second-to-third',
        from: 'second',
        to: 'third',
        condition: { kind: 'always' },
      },
    ]);
  });

  test('rejects staged elements with a source-located feature error', () => {
    const source = validWorkflow(`
    <parallel id="work">
      <step id="a"><script>return 1;</script></step>
    </parallel>`);
    const error = validationError(source);

    expect(error.diagnostic.code).toBe('WOML_FEATURE_NOT_EXECUTABLE');
    expect(error.diagnostic.phase).toBe('validation');
    expect(error.diagnostic.location.start.offset).toBe(source.indexOf('<parallel'));
    expect(error.diagnostic.message).toContain('<parallel>');
  });

  test('rejects staged attributes instead of silently ignoring them', () => {
    const source = validWorkflow(
      '<step id="a" retry="1"><script>return 1;</script></step>',
    );
    const error = validationError(source);

    expect(error.diagnostic.code).toBe('WOML_FEATURE_NOT_EXECUTABLE');
    expect(error.diagnostic.location.start.offset).toBe(source.indexOf('retry'));
  });

  test('rejects unknown dotted capability elements cleanly', () => {
    const source = validWorkflow(
      '<step id="a"><db.insert table="users" /></step>',
    );
    const error = validationError(source);

    expect(error.diagnostic.code).toBe('WOML_UNKNOWN_ELEMENT');
    expect(error.diagnostic.message).toContain('<db.insert>');
    expect(error.diagnostic.location.start.offset).toBe(source.indexOf('<db.insert'));
  });

  test('rejects unknown attributes at the attribute name', () => {
    const source = validWorkflow(
      '<step id="a" surprise="yes"><script>return 1;</script></step>',
    );
    const error = validationError(source);

    expect(error.diagnostic.code).toBe('WOML_UNKNOWN_ATTRIBUTE');
    expect(error.diagnostic.location.start.offset).toBe(source.indexOf('surprise'));
  });

  test('rejects empty steps', () => {
    const source = validWorkflow('');
    const error = validationError(source);

    expect(error.diagnostic.code).toBe('WOML_EMPTY_STEPS');
    expect(error.diagnostic.message).toContain('at least one <step>');
  });

  test('rejects invalid workflow and step IDs at their values', () => {
    const workflowSource = validWorkflow().replace(
      'id="test-workflow"',
      'id="Test Workflow"',
    );
    const workflowError = validationError(workflowSource);
    expect(workflowError.diagnostic.code).toBe('WOML_INVALID_ID');
    expect(workflowError.diagnostic.location.start.offset).toBe(
      workflowSource.indexOf('Test Workflow'),
    );

    const stepSource = validWorkflow(
      '<step id="invalid-step"><script>return 1;</script></step>',
    );
    const stepError = validationError(stepSource);
    expect(stepError.diagnostic.code).toBe('WOML_INVALID_ID');
    expect(stepError.diagnostic.location.start.offset).toBe(
      stepSource.indexOf('invalid-step'),
    );
  });

  test('rejects duplicate step IDs at the second ID', () => {
    const source = validWorkflow(`
    <step id="same"><script>return 1;</script></step>
    <step id="same"><script>return 2;</script></step>`);
    const error = validationError(source);
    const secondIdOffset = source.lastIndexOf('same');

    expect(error.diagnostic.code).toBe('WOML_DUPLICATE_ID');
    expect(error.diagnostic.location.start.offset).toBe(secondIdOffset);
  });

  test('rejects missing required children and attributes', () => {
    const missingTriggers = `<workflow woml-version="0.1" id="test-workflow">
  <steps><step id="a"><script>return 1;</script></step></steps>
</workflow>`;
    expect(validationError(missingTriggers).diagnostic.code).toBe(
      'WOML_TRIGGER_CONTAINER_COUNT',
    );

    const missingStepId = validWorkflow(
      '<step><script>return 1;</script></step>',
    );
    expect(validationError(missingStepId).diagnostic.code).toBe(
      'WOML_MISSING_ATTRIBUTE',
    );
  });

  test('requires one manual trigger and the canonical container order', () => {
    const multipleManual = `<workflow woml-version="0.1" id="test-workflow">
  <triggers><manual id="first" /><manual id="second" /></triggers>
  <steps><step id="a"><script>return 1;</script></step></steps>
</workflow>`;
    expect(validationError(multipleManual).diagnostic.code).toBe(
      'WOML_MANUAL_TRIGGER_COUNT',
    );

    const wrongOrder = `<workflow woml-version="0.1" id="test-workflow">
  <steps><step id="a"><script>return 1;</script></step></steps>
  <triggers><manual id="start" /></triggers>
</workflow>`;
    expect(validationError(wrongOrder).diagnostic.code).toBe(
      'WOML_INVALID_STRUCTURE',
    );
  });

  test('requires the workflow root and validates trigger IDs', () => {
    const wrongRoot = '<steps><step id="a"><script>return 1;</script></step></steps>';
    expect(validationError(wrongRoot).diagnostic.code).toBe(
      'WOML_EXPECTED_WORKFLOW_ROOT',
    );

    const invalidTrigger = validWorkflow().replace('id="start"', 'id="bad-trigger"');
    expect(validationError(invalidTrigger).diagnostic.code).toBe('WOML_INVALID_ID');
  });

  test('rejects multiple step operations', () => {
    const source = validWorkflow(
      '<step id="a"><script>return 1;</script><script>return 2;</script></step>',
    );
    const error = validationError(source);

    expect(error.diagnostic.code).toBe('WOML_STEP_OPERATION_COUNT');
    expect(error.diagnostic.location.start.offset).toBe(
      source.lastIndexOf('<script>'),
    );
  });

  test('rejects empty metadata and unsupported WOML versions', () => {
    const emptyName = validWorkflow(
      '<step id="a" name="   "><script>return 1;</script></step>',
    );
    expect(validationError(emptyName).diagnostic.code).toBe(
      'WOML_EMPTY_METADATA',
    );

    const futureVersion = validWorkflow().replace(
      'woml-version="0.1"',
      'woml-version="0.2"',
    );
    expect(validationError(futureVersion).diagnostic.code).toBe(
      'WOML_UNSUPPORTED_VERSION',
    );
  });
});

describe('inspectCompiledWorkflowGraph', () => {
  test('reports graph-boundary failures without knowing WOML syntax', () => {
    const graph: CompiledWorkflowGraph = {
      entryNodeIds: ['a'],
      nodes: [
        { id: 'a', handler: 'runtime.script', inputs: { kind: 'literal', value: null } },
        { id: 'b', handler: 'runtime.script', inputs: { kind: 'literal', value: null } },
        { id: 'c', handler: 'runtime.script', inputs: { kind: 'literal', value: null } },
      ],
      edges: [
        { id: 'cycle', from: 'a', to: 'b', condition: { kind: 'always' } },
        { id: 'cycle', from: 'b', to: 'a', condition: { kind: 'always' } },
        { id: 'missing', from: 'b', to: 'missing', condition: { kind: 'always' } },
      ],
    };

    const codes = inspectCompiledWorkflowGraph(graph).map((issue) => issue.code);
    expect(codes).toContain('DUPLICATE_EDGE_ID');
    expect(codes).toContain('UNKNOWN_EDGE_ENDPOINT');
    expect(codes).toContain('INVALID_ENTRY_NODE');
    expect(codes).toContain('UNREACHABLE_NODE');
    expect(codes).toContain('CYCLIC_GRAPH');
  });

  test('enforces the first CLI profile single-terminal rule separately', () => {
    const graph: CompiledWorkflowGraph = {
      entryNodeIds: ['a', 'b'],
      nodes: [
        { id: 'a', handler: 'runtime.script', inputs: { kind: 'literal', value: null } },
        { id: 'b', handler: 'runtime.script', inputs: { kind: 'literal', value: null } },
      ],
      edges: [],
    };

    expect(
      inspectCompiledWorkflowGraph(graph, { requireSingleTerminal: true }).map(
        (issue) => issue.code,
      ),
    ).toContain('TERMINAL_NODE_COUNT');
  });
});
