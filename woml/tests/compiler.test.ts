import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

import {
  compileWoml,
  inspectCompiledWorkflowGraph,
  parseWoml,
  validateWoml,
  WomlCompileError,
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

function compileError(source: string): WomlCompileError {
  try {
    compile(source);
  } catch (error) {
    if (error instanceof WomlCompileError) return error;
    throw error;
  }
  throw new Error('Expected WOML compilation to fail.');
}

function validWorkflow(
  stepMarkup = '<step id="a"><script>return { ok: true };</script></step>'
) {
  return `<workflow version="1.0.0" id="test-workflow">
  <triggers><manual id="start" /></triggers>
  <steps>${stepMarkup}</steps>
</workflow>`;
}

describe('compileWoml', () => {
  test('A2 lowers the reviewed approval fixture exactly to model v4', () => {
    const source = readFileSync(
      new URL('./fixtures/approval.woml', import.meta.url),
      'utf8'
    );
    const expected = JSON.parse(
      readFileSync(
        new URL('./fixtures/approval.compiled.v4.json', import.meta.url),
        'utf8'
      )
    );
    const document = parseWoml(source, { file: 'approval.woml' });

    expect(() => validateWoml(document)).not.toThrow();
    const compiled = compileWoml(document);

    expect(compiled).toEqual(expected);
    expect(compiled.schemaVersion).toBe(4);
    expect(inspectCompiledWorkflowGraph(compiled.graph)).toEqual([]);
  });

  test('lowers empty approval arms directly to the deterministic join', () => {
    const compiled = compile(
      validWorkflow(`
      <approval id="review">
        <when-approved />
        <when-rejected />
      </approval>`)
    );

    expect(compiled.schemaVersion).toBe(4);
    expect(compiled.graph.nodes.map(node => node.id)).toEqual([
      'review',
      '__woml_approval__review__join',
    ]);
    expect(compiled.graph.edges).toEqual([
      {
        id: 'review:approved',
        from: 'review',
        to: '__woml_approval__review__join',
        condition: {
          kind: 'equals',
          left: {
            kind: 'contextReference',
            path: ['steps', 'review', 'decision'],
          },
          right: { kind: 'literal', value: 'approved' },
        },
        approvalId: 'review',
      },
      {
        id: 'review:rejected',
        from: 'review',
        to: '__woml_approval__review__join',
        condition: {
          kind: 'equals',
          left: {
            kind: 'contextReference',
            path: ['steps', 'review', 'decision'],
          },
          right: { kind: 'literal', value: 'rejected' },
        },
        approvalId: 'review',
      },
    ]);
    expect(inspectCompiledWorkflowGraph(compiled.graph)).toEqual([]);
  });

  test('lowers nested approvals and branch composition as one valid v4 DAG', () => {
    const compiled = compile(
      validWorkflow(`
      <step id="ready"><script>return { ok: true };</script></step>
      <branch id="route">
        <when test="{{context.steps.ready.ok}}">
          <approval id="outer">
            <when-approved>
              <approval id="inner">
                <when-approved />
                <when-rejected />
              </approval>
            </when-approved>
            <when-rejected />
          </approval>
          <result value="{{context.steps.outer}}" />
        </when>
        <otherwise>
          <step id="fallback"><script>return false;</script></step>
          <result value="{{context.steps.fallback}}" />
        </otherwise>
      </branch>
      <step id="finish"><script>return context.steps.route;</script></step>`)
    );

    expect(compiled.schemaVersion).toBe(4);
    expect(inspectCompiledWorkflowGraph(compiled.graph)).toEqual([]);
    expect(
      compiled.graph.nodes.filter(node =>
        node.handler.startsWith('engine.approval-')
      )
    ).toHaveLength(4);
  });

  test('accepts empty arms, nested approvals, and approval output references inside decision arms', () => {
    const source = validWorkflow(`
    <step id="calculate"><script>return { ready: true };</script></step>
    <approval id="outerApproval" timeout="1.5h">
      <when-approved>
        <branch id="approvedRoute">
          <when test="{{context.steps.calculate.ready}}">
            <approval id="nestedApproval">
              <when-approved></when-approved>
              <when-rejected />
            </approval>
            <result value="{{context.steps.nestedApproval}}" />
          </when>
          <otherwise>
            <step id="fallback"><script>return false;</script></step>
            <result value="{{context.steps.outerApproval}}" />
          </otherwise>
        </branch>
      </when-approved>
      <when-rejected />
    </approval>
    <step id="finish"><script>return context.steps.outerApproval;</script></step>`);

    expect(() =>
      validateWoml(parseWoml(source, { file: 'workflow.woml' }))
    ).not.toThrow();
  });

  test('requires exactly ordered approval arms', () => {
    const missing = validWorkflow(`
    <approval id="review">
      <when-approved />
    </approval>`);
    expect(validationError(missing).diagnostic.code).toBe(
      'WOML_APPROVAL_STRUCTURE_INVALID'
    );

    const duplicate = validWorkflow(`
    <approval id="review">
      <when-approved />
      <when-rejected />
      <when-rejected />
    </approval>`);
    const duplicateError = validationError(duplicate);
    expect(duplicateError.diagnostic.code).toBe(
      'WOML_APPROVAL_STRUCTURE_INVALID'
    );
    expect(duplicateError.diagnostic.location.start.offset).toBe(
      duplicate.lastIndexOf('<when-rejected')
    );

    const reversed = validWorkflow(`
    <approval id="review">
      <when-rejected />
      <when-approved />
    </approval>`);
    const reversedError = validationError(reversed);
    expect(reversedError.diagnostic.code).toBe(
      'WOML_APPROVAL_STRUCTURE_INVALID'
    );
    expect(reversedError.diagnostic.location.start.offset).toBe(
      reversed.indexOf('<when-rejected')
    );

    const directStep = validWorkflow(`
    <approval id="review">
      <step id="wrong"><script>return 1;</script></step>
      <when-approved />
      <when-rejected />
    </approval>`);
    expect(validationError(directStep).diagnostic.code).toBe(
      'WOML_APPROVAL_STRUCTURE_INVALID'
    );
  });

  test('validates approval duration and timeout policy without ambiguity', () => {
    for (const timeout of ['500ms', '0.5s', '30m', '24h', '2d']) {
      const source = validWorkflow(`
      <approval id="review" timeout="${timeout}">
        <when-approved />
        <when-rejected />
      </approval>`);
      expect(() =>
        validateWoml(parseWoml(source, { file: 'workflow.woml' }))
      ).not.toThrow();
    }

    for (const timeout of [
      '0ms',
      '0.1ms',
      '24',
      '-1h',
      '1w',
      '1e3s',
      '999999999999999999999d',
    ]) {
      const source = validWorkflow(`
      <approval id="review" timeout="${timeout}">
        <when-approved />
        <when-rejected />
      </approval>`);
      expect(validationError(source).diagnostic.code).toBe(
        'WOML_APPROVAL_TIMEOUT_INVALID'
      );
    }

    const policyWithoutTimeout = validWorkflow(`
    <approval id="review" on-timeout="reject">
      <when-approved />
      <when-rejected />
    </approval>`);
    expect(validationError(policyWithoutTimeout).diagnostic.code).toBe(
      'WOML_APPROVAL_TIMEOUT_INVALID'
    );

    const invalidPolicy = validWorkflow(`
    <approval id="review" timeout="1h" on-timeout="approve">
      <when-approved />
      <when-rejected />
    </approval>`);
    expect(validationError(invalidPolicy).diagnostic.code).toBe(
      'WOML_APPROVAL_TIMEOUT_INVALID'
    );
  });

  test('shares approval IDs with the workflow structural namespace', () => {
    const source = validWorkflow(`
    <step id="review"><script>return true;</script></step>
    <approval id="review">
      <when-approved />
      <when-rejected />
    </approval>`);
    const error = validationError(source);

    expect(error.diagnostic.code).toBe('WOML_DUPLICATE_ID');
    expect(error.diagnostic.location.start.offset).toBe(
      source.indexOf('review', source.indexOf('<approval'))
    );
  });

  test('rejects approval-only children outside approval and approval inside parallel', () => {
    const looseArm = validWorkflow('<when-approved />');
    expect(validationError(looseArm).diagnostic.code).toBe(
      'WOML_APPROVAL_PLACEMENT_INVALID'
    );

    const parallelApproval = validWorkflow(`
    <parallel id="checks">
      <approval id="review">
        <when-approved />
        <when-rejected />
      </approval>
    </parallel>
    <step id="finish"><script>return true;</script></step>`);
    const placementError = validationError(parallelApproval);
    expect(placementError.diagnostic.code).toBe(
      'WOML_APPROVAL_PLACEMENT_INVALID'
    );
    expect(placementError.diagnostic.location.start.offset).toBe(
      parallelApproval.indexOf('<approval')
    );
  });

  test('rejects executable scripts as notification providers', () => {
    const source = validWorkflow(`
    <approval id="review">
      <notify><script>return { sent: true };</script></notify>
      <when-approved />
      <when-rejected />
    </approval>`);
    const error = validationError(source);

    expect(error.diagnostic.code).toBe('WOML_NOTIFY_UNSUPPORTED_PROVIDER');
    expect(error.diagnostic.location.start.offset).toBe(
      source.indexOf('<script')
    );
  });

  test('deep-equals the reviewed Phase 0 compiled fixture', () => {
    const source = readFileSync(
      new URL('./fixtures/hello.woml', import.meta.url),
      'utf8'
    );
    const expected = JSON.parse(
      readFileSync(
        new URL('./fixtures/hello.compiled.v1.json', import.meta.url),
        'utf8'
      )
    );

    const compiled = compileWoml(parseWoml(source, { file: 'hello.woml' }));

    expect(compiled).toEqual(expected);
    expect(
      inspectCompiledWorkflowGraph(compiled.graph, {
        requireSingleTerminal: true,
      })
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
    <step id="third"><script>return 3;</script></step>`)
    );

    expect(compiled.graph.entryNodeIds).toEqual(['first']);
    expect(compiled.graph.nodes.map(node => node.id)).toEqual([
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

  test('lowers the reviewed parallel fixture exactly to compiled model v3', () => {
    const source = readFileSync(
      new URL('./fixtures/parallel.woml', import.meta.url),
      'utf8'
    );
    const expected = JSON.parse(
      readFileSync(
        new URL('./fixtures/parallel.compiled.v3.json', import.meta.url),
        'utf8'
      )
    );
    const compiled = compileWoml(parseWoml(source, { file: 'parallel.woml' }));

    expect(compiled).toEqual(expected);
    expect(compiled.schemaVersion).toBe(3);
    expect(inspectCompiledWorkflowGraph(compiled.graph)).toEqual([]);
  });

  test('lowers one-child groups with the frozen default policy and cap', () => {
    const source = validWorkflow(`
    <parallel id="one" name="One child" description="Degenerate group">
      <step id="child"><script>return 1;</script></step>
    </parallel>
    <step id="result"><script>return context.steps.child;</script></step>`);

    const compiled = compile(source);
    expect(compiled.schemaVersion).toBe(3);
    expect(inspectCompiledWorkflowGraph(compiled.graph)).toEqual([]);
    expect(
      compiled.graph.nodes.find(
        node => node.handler === 'engine.parallel-start'
      )?.inputs
    ).toEqual({
      kind: 'object',
      fields: {
        concurrency: { kind: 'literal', value: 1 },
        onError: { kind: 'literal', value: 'fail-fast' },
      },
    });
  });

  test('lowers parallel inside a branch arm into one valid model-v3 DAG', () => {
    const source = validWorkflow(`
    <step id="ready"><script>return true;</script></step>
    <branch id="route">
      <when test="{{context.steps.ready}}">
        <parallel id="checks" concurrency="2" on-error="wait-all">
          <step id="left"><script>return 1;</script></step>
          <step id="right"><script>return 2;</script></step>
        </parallel>
        <result value="{{context.steps.left}}" />
      </when>
      <otherwise>
        <step id="fallback"><script>return 0;</script></step>
        <result value="{{context.steps.fallback}}" />
      </otherwise>
    </branch>`);

    const compiled = compile(source);
    expect(compiled.schemaVersion).toBe(3);
    expect(inspectCompiledWorkflowGraph(compiled.graph)).toEqual([]);
    expect(compiled.graph.nodes.map(node => node.id)).toContain(
      '__woml_parallel__checks__start'
    );
    expect(
      compiled.graph.edges.filter(edge => edge.parallelId === 'checks')
    ).toHaveLength(4);
  });

  test('rejects empty and unsupported parallel children at their source', () => {
    const emptySource = validWorkflow(`
    <parallel id="empty"></parallel>
    <step id="result"><script>return 1;</script></step>`);
    expect(validationError(emptySource).diagnostic.code).toBe(
      'WOML_PARALLEL_EMPTY'
    );

    const nestedSource = validWorkflow(`
    <parallel id="outer">
      <branch id="inner">
        <when test="{{context.trigger.ok}}">
          <step id="yes"><script>return 1;</script></step>
          <result value="{{context.steps.yes}}" />
        </when>
        <otherwise>
          <step id="no"><script>return 0;</script></step>
          <result value="{{context.steps.no}}" />
        </otherwise>
      </branch>
    </parallel>
    <step id="result"><script>return 1;</script></step>`);
    const nestedError = validationError(nestedSource);
    expect(nestedError.diagnostic.code).toBe('WOML_PARALLEL_CHILD_UNSUPPORTED');
    expect(nestedError.diagnostic.location.start.offset).toBe(
      nestedSource.indexOf('<branch')
    );
  });

  test('validates parallel concurrency and failure policy', () => {
    for (const value of ['0', '-1', '1.5', 'two']) {
      const source = validWorkflow(`
      <parallel id="group" concurrency="${value}">
        <step id="child"><script>return 1;</script></step>
      </parallel>
      <step id="result"><script>return 1;</script></step>`);
      expect(validationError(source).diagnostic.code).toBe(
        'WOML_PARALLEL_INVALID_CONCURRENCY'
      );
    }

    const excessive = validWorkflow(`
    <parallel id="group" concurrency="3">
      <step id="left"><script>return 1;</script></step>
      <step id="right"><script>return 2;</script></step>
    </parallel>
    <step id="result"><script>return 1;</script></step>`);
    expect(validationError(excessive).diagnostic.code).toBe(
      'WOML_PARALLEL_INVALID_CONCURRENCY'
    );

    const badPolicy = validWorkflow(`
    <parallel id="group" on-error="continue">
      <step id="child"><script>return 1;</script></step>
    </parallel>
    <step id="result"><script>return 1;</script></step>`);
    expect(validationError(badPolicy).diagnostic.code).toBe(
      'WOML_PARALLEL_INVALID_POLICY'
    );
  });

  test('validates parallel identity, metadata, and attributes', () => {
    const missingId = validWorkflow(`
    <parallel>
      <step id="child"><script>return 1;</script></step>
    </parallel>
    <step id="result"><script>return 1;</script></step>`);
    expect(validationError(missingId).diagnostic.code).toBe(
      'WOML_MISSING_ATTRIBUTE'
    );

    const invalidId = validWorkflow(`
    <parallel id="bad-id">
      <step id="child"><script>return 1;</script></step>
    </parallel>
    <step id="result"><script>return 1;</script></step>`);
    expect(validationError(invalidId).diagnostic.code).toBe('WOML_INVALID_ID');

    const emptyName = validWorkflow(`
    <parallel id="group" name="  ">
      <step id="child"><script>return 1;</script></step>
    </parallel>
    <step id="result"><script>return 1;</script></step>`);
    expect(validationError(emptyName).diagnostic.code).toBe(
      'WOML_EMPTY_METADATA'
    );

    const unknownAttribute = validWorkflow(`
    <parallel id="group" mode="fast">
      <step id="child"><script>return 1;</script></step>
    </parallel>
    <step id="result"><script>return 1;</script></step>`);
    expect(validationError(unknownAttribute).diagnostic.code).toBe(
      'WOML_UNKNOWN_ATTRIBUTE'
    );
  });

  test('keeps parallel, branch, and step IDs in one namespace', () => {
    const source = validWorkflow(`
    <step id="shared"><script>return 1;</script></step>
    <parallel id="shared">
      <step id="child"><script>return 2;</script></step>
    </parallel>
    <step id="result"><script>return 1;</script></step>`);
    const error = validationError(source);

    expect(error.diagnostic.code).toBe('WOML_DUPLICATE_ID');
    expect(error.diagnostic.location.start.offset).toBe(
      source.indexOf('shared', source.indexOf('<parallel'))
    );
  });

  test('rejects a root terminal parallel because it has no aggregate result', () => {
    const source = validWorkflow(`
    <parallel id="finalGroup">
      <step id="child"><script>return 1;</script></step>
    </parallel>`);
    const error = validationError(source);

    expect(error.diagnostic.code).toBe('WOML_PARALLEL_TERMINAL_UNSUPPORTED');
    expect(error.diagnostic.location.start.offset).toBe(
      source.indexOf('<parallel')
    );
  });

  test('parallel IDs are structural only while child outputs dominate downstream references', () => {
    const invalidSource = validWorkflow(`
    <parallel id="group">
      <step id="child"><script>return true;</script></step>
    </parallel>
    <branch id="route">
      <when test="{{context.steps.group}}">
        <step id="yes"><script>return 1;</script></step>
        <result value="{{context.steps.yes}}" />
      </when>
      <otherwise>
        <step id="no"><script>return 0;</script></step>
        <result value="{{context.steps.no}}" />
      </otherwise>
    </branch>`);
    expect(compileError(invalidSource).diagnostic.code).toBe(
      'WOML_REFERENCE_NOT_DOMINATING'
    );

    const validSource = invalidSource.replace(
      '{{context.steps.group}}',
      '{{context.steps.child}}'
    );
    const compiled = compile(validSource);
    expect(compiled.schemaVersion).toBe(3);
    expect(inspectCompiledWorkflowGraph(compiled.graph)).toEqual([]);
  });

  test('rejects the remaining staged timeout attribute instead of silently ignoring it', () => {
    const source = validWorkflow(
      '<step id="a" timeout="1s"><script>return 1;</script></step>'
    );
    const error = validationError(source);

    expect(error.diagnostic.code).toBe('WOML_FEATURE_NOT_EXECUTABLE');
    expect(error.diagnostic.location.start.offset).toBe(
      source.indexOf('timeout')
    );
  });

  test('RI1 lowers the reviewed retry fixture exactly to model v6', () => {
    const source = readFileSync(
      new URL('./fixtures/retry.woml', import.meta.url),
      'utf8'
    );
    const expected = JSON.parse(
      readFileSync(
        new URL('./fixtures/retry.compiled.v6.json', import.meta.url),
        'utf8'
      )
    );
    const compiled = compile(source);

    expect(compiled).toEqual(expected);
    expect(compiled.schemaVersion).toBe(6);
    expect(inspectCompiledWorkflowGraph(compiled.graph)).toEqual([]);
    expect(compile(source)).toEqual(compiled);
  });

  test('treats omitted retry and retry="1" as the same older-model behavior', () => {
    const omitted = compile(
      validWorkflow('<step id="a"><script>return 1;</script></step>')
    );
    const one = compile(
      validWorkflow('<step id="a" retry="1"><script>return 1;</script></step>')
    );

    expect(one).toEqual(omitted);
    expect(one.schemaVersion).toBe(1);
    expect(one.graph.nodes[0].retryPolicy).toBeUndefined();
  });

  test('applies frozen retry defaults and fixed-backoff lowering', () => {
    const exponential = compile(
      validWorkflow('<step id="a" retry="3"><script>return 1;</script></step>')
    );
    expect(exponential.schemaVersion).toBe(6);
    expect(exponential.graph.nodes[0].retryPolicy).toEqual({
      maxAttempts: 3,
      backoff: {
        kind: 'exponential',
        initialDelayMs: 1000,
        multiplier: 2,
        maximumDelayMs: 30000,
      },
    });

    const fixed = compile(
      validWorkflow(
        '<step id="a" retry="2" retry-backoff="fixed" retry-delay="250ms"><script>return 1;</script></step>'
      )
    );
    expect(fixed.graph.nodes[0].retryPolicy).toEqual({
      maxAttempts: 2,
      backoff: { kind: 'fixed', delayMs: 250 },
    });
  });

  test('accepts retry on normal steps nested in branch, parallel, and approval routes', () => {
    const branch = compile(
      validWorkflow(`
      <step id="ready"><script>return true;</script></step>
      <branch id="route">
        <when test="{{context.steps.ready}}">
          <step id="selected" retry="2"><script>return 1;</script></step>
          <result value="{{context.steps.selected}}" />
        </when>
        <otherwise>
          <step id="fallback"><script>return 0;</script></step>
          <result value="{{context.steps.fallback}}" />
        </otherwise>
      </branch>`)
    );
    expect(branch.schemaVersion).toBe(6);

    const parallel = compile(
      validWorkflow(`
      <parallel id="group">
        <step id="child" retry="2"><script>return 1;</script></step>
      </parallel>
      <step id="finish"><script>return context.steps.child;</script></step>`)
    );
    expect(parallel.schemaVersion).toBe(6);

    const approval = compile(
      validWorkflow(`
      <approval id="review">
        <when-approved>
          <step id="publish" retry="2"><script>return 1;</script></step>
        </when-approved>
        <when-rejected />
      </approval>`)
    );
    expect(approval.schemaVersion).toBe(6);
  });

  test('reports frozen retry diagnostics at the responsible attribute', () => {
    const cases = [
      {
        markup: '<step id="a" retry="0"><script>return 1;</script></step>',
        code: 'WOML_RETRY_INVALID',
        token: '0',
      },
      {
        markup: '<step id="a" retry="11"><script>return 1;</script></step>',
        code: 'WOML_RETRY_INVALID',
        token: '11',
      },
      {
        markup:
          '<step id="a" retry-backoff="fixed"><script>return 1;</script></step>',
        code: 'WOML_RETRY_BACKOFF_REQUIRES_RETRY',
        token: 'retry-backoff',
      },
      {
        markup:
          '<step id="a" retry="2" retry-backoff="random"><script>return 1;</script></step>',
        code: 'WOML_RETRY_BACKOFF_INVALID',
        token: 'random',
      },
      {
        markup:
          '<step id="a" retry="2" retry-delay="0s"><script>return 1;</script></step>',
        code: 'WOML_RETRY_DELAY_INVALID',
        token: '0s',
      },
      {
        markup:
          '<step id="a" retry="2" retry-delay="2s" retry-max-delay="1s"><script>return 1;</script></step>',
        code: 'WOML_RETRY_MAX_DELAY_INVALID',
        token: '1s',
      },
      {
        markup:
          '<step id="a" retry="2" retry-backoff="fixed" retry-max-delay="1s"><script>return 1;</script></step>',
        code: 'WOML_RETRY_MAX_DELAY_NOT_ALLOWED',
        token: 'retry-max-delay',
      },
    ];

    for (const entry of cases) {
      const source = validWorkflow(entry.markup);
      const error = validationError(source);
      expect(error.diagnostic.code).toBe(entry.code);
      expect(error.diagnostic.location.start.offset).toBe(
        source.indexOf(entry.token, source.indexOf('<step'))
      );
    }
  });

  test('keeps retry attributes step-only and rejects a retry element', () => {
    const structural = validWorkflow(`
    <parallel id="group" retry="2">
      <step id="child"><script>return 1;</script></step>
    </parallel>
    <step id="finish"><script>return 1;</script></step>`);
    expect(validationError(structural).diagnostic.code).toBe(
      'WOML_RETRY_HANDLER_UNSUPPORTED'
    );

    const element = validWorkflow(
      '<step id="a"><retry /><script>return 1;</script></step>'
    );
    expect(validationError(element).diagnostic.code).toBe(
      'WOML_UNKNOWN_ELEMENT'
    );
  });

  test('rejects unknown dotted capability elements cleanly', () => {
    const source = validWorkflow(
      '<step id="a"><db.insert table="users" /></step>'
    );
    const error = validationError(source);

    expect(error.diagnostic.code).toBe('WOML_UNKNOWN_ELEMENT');
    expect(error.diagnostic.message).toContain('<db.insert>');
    expect(error.diagnostic.location.start.offset).toBe(
      source.indexOf('<db.insert')
    );
  });

  test('rejects unknown attributes at the attribute name', () => {
    const source = validWorkflow(
      '<step id="a" surprise="yes"><script>return 1;</script></step>'
    );
    const error = validationError(source);

    expect(error.diagnostic.code).toBe('WOML_UNKNOWN_ATTRIBUTE');
    expect(error.diagnostic.location.start.offset).toBe(
      source.indexOf('surprise')
    );
  });

  test('rejects empty steps', () => {
    const source = validWorkflow('');
    const error = validationError(source);

    expect(error.diagnostic.code).toBe('WOML_EMPTY_STEPS');
    expect(error.diagnostic.message).toContain('at least one step item');
  });

  test('rejects invalid workflow and step IDs at their values', () => {
    const workflowSource = validWorkflow().replace(
      'id="test-workflow"',
      'id="Test Workflow"'
    );
    const workflowError = validationError(workflowSource);
    expect(workflowError.diagnostic.code).toBe('WOML_INVALID_ID');
    expect(workflowError.diagnostic.location.start.offset).toBe(
      workflowSource.indexOf('Test Workflow')
    );

    const stepSource = validWorkflow(
      '<step id="invalid-step"><script>return 1;</script></step>'
    );
    const stepError = validationError(stepSource);
    expect(stepError.diagnostic.code).toBe('WOML_INVALID_ID');
    expect(stepError.diagnostic.location.start.offset).toBe(
      stepSource.indexOf('invalid-step')
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

  test('lowers the reviewed branch fixture exactly to compiled model v2', () => {
    const source = readFileSync(
      new URL('./fixtures/branch.woml', import.meta.url),
      'utf8'
    );
    const expected = JSON.parse(
      readFileSync(
        new URL('./fixtures/branch.compiled.v2.json', import.meta.url),
        'utf8'
      )
    );
    const compiled = compile(source);

    expect(compiled).toEqual(expected);
    expect(compiled.schemaVersion).toBe(2);
    expect(inspectCompiledWorkflowGraph(compiled.graph)).toEqual([]);
    expect(compile(source)).toEqual(compiled);
  });

  test('lowers recursively nested branches and route-local results into one valid DAG', () => {
    const source = validWorkflow(`
    <step id="ready"><script>return true;</script></step>
    <branch id="outer">
      <when test="{{context.steps.ready}}">
        <branch id="inner">
          <when test="{{context.steps.ready}}">
            <step id="inside"><script>return { ok: true };</script></step>
            <result value="{{context.steps.inside}}" />
          </when>
          <otherwise>
            <step id="innerFallback"><script>return { ok: false };</script></step>
            <result value="{{context.steps.innerFallback}}" />
          </otherwise>
        </branch>
        <result value="{{context.steps.inner}}" />
      </when>
      <otherwise>
        <step id="outerFallback"><script>return { ok: false };</script></step>
        <result value="{{context.steps.outerFallback}}" />
      </otherwise>
    </branch>`);

    const compiled = compile(source);
    expect(compiled.schemaVersion).toBe(2);
    expect(inspectCompiledWorkflowGraph(compiled.graph)).toEqual([]);
    expect(compiled.graph.entryNodeIds).toEqual(['ready']);
    expect(compiled.graph.nodes.map(node => node.id)).toContain('inner');
    expect(compiled.graph.nodes.map(node => node.id)).toContain('outer');
  });

  test('enforces required branch cases and their order', () => {
    const noWhen = validWorkflow(`
    <branch id="decision">
      <otherwise>
        <step id="fallback"><script>return 0;</script></step>
        <result value="{{context.steps.fallback}}" />
      </otherwise>
    </branch>`);
    expect(validationError(noWhen).diagnostic.code).toBe(
      'WOML_BRANCH_WHEN_REQUIRED'
    );

    const noOtherwise = validWorkflow(`
    <step id="ready"><script>return true;</script></step>
    <branch id="decision">
      <when test="{{context.steps.ready}}">
        <step id="selected"><script>return 1;</script></step>
        <result value="{{context.steps.selected}}" />
      </when>
    </branch>`);
    expect(validationError(noOtherwise).diagnostic.code).toBe(
      'WOML_BRANCH_OTHERWISE_REQUIRED'
    );

    const misplacedOtherwise = validWorkflow(`
    <step id="ready"><script>return true;</script></step>
    <branch id="decision">
      <otherwise>
        <step id="fallback"><script>return 0;</script></step>
        <result value="{{context.steps.fallback}}" />
      </otherwise>
      <when test="{{context.steps.ready}}">
        <step id="selected"><script>return 1;</script></step>
        <result value="{{context.steps.selected}}" />
      </when>
    </branch>`);
    const misplacedError = validationError(misplacedOtherwise);
    expect(misplacedError.diagnostic.code).toBe('WOML_BRANCH_OTHERWISE_ORDER');
    expect(misplacedError.diagnostic.location.start.offset).toBe(
      misplacedOtherwise.indexOf('<otherwise>')
    );

    const duplicateOtherwise = validWorkflow(`
    <step id="ready"><script>return true;</script></step>
    <branch id="decision">
      <when test="{{context.steps.ready}}">
        <step id="selected"><script>return 1;</script></step>
        <result value="{{context.steps.selected}}" />
      </when>
      <otherwise>
        <step id="firstFallback"><script>return 0;</script></step>
        <result value="{{context.steps.firstFallback}}" />
      </otherwise>
      <otherwise>
        <step id="secondFallback"><script>return 0;</script></step>
        <result value="{{context.steps.secondFallback}}" />
      </otherwise>
    </branch>`);
    const duplicateError = validationError(duplicateOtherwise);
    expect(duplicateError.diagnostic.code).toBe('WOML_BRANCH_OTHERWISE_ORDER');
    expect(duplicateError.diagnostic.location.start.offset).toBe(
      duplicateOtherwise.lastIndexOf('<otherwise>')
    );
  });

  test('requires exactly one final result after at least one arm item', () => {
    const missingResult = validWorkflow(`
    <step id="ready"><script>return true;</script></step>
    <branch id="decision">
      <when test="{{context.steps.ready}}">
        <step id="selected"><script>return 1;</script></step>
      </when>
      <otherwise>
        <step id="fallback"><script>return 0;</script></step>
        <result value="{{context.steps.fallback}}" />
      </otherwise>
    </branch>`);
    expect(validationError(missingResult).diagnostic.code).toBe(
      'WOML_BRANCH_RESULT_REQUIRED'
    );

    const duplicateResult = missingResult.replace(
      '<step id="selected"><script>return 1;</script></step>',
      `<step id="selected"><script>return 1;</script></step>
        <result value="{{context.steps.selected}}" />
        <result value="{{context.steps.selected}}" />`
    );
    const duplicateResultError = validationError(duplicateResult);
    expect(duplicateResultError.diagnostic.code).toBe(
      'WOML_BRANCH_RESULT_REQUIRED'
    );
    expect(duplicateResultError.diagnostic.location.start.offset).toBe(
      duplicateResult.indexOf('<result', duplicateResult.indexOf('<result') + 1)
    );

    const misplacedResult = validWorkflow(`
    <step id="ready"><script>return true;</script></step>
    <branch id="decision">
      <when test="{{context.steps.ready}}">
        <result value="{{context.steps.ready}}" />
        <step id="selected"><script>return 1;</script></step>
      </when>
      <otherwise>
        <step id="fallback"><script>return 0;</script></step>
        <result value="{{context.steps.fallback}}" />
      </otherwise>
    </branch>`);
    const orderError = validationError(misplacedResult);
    expect(orderError.diagnostic.code).toBe('WOML_BRANCH_RESULT_ORDER');
    expect(orderError.diagnostic.location.start.offset).toBe(
      misplacedResult.indexOf('<result')
    );

    const emptyArm = validWorkflow(`
    <step id="ready"><script>return true;</script></step>
    <branch id="decision">
      <when test="{{context.steps.ready}}">
        <result value="{{context.steps.ready}}" />
      </when>
      <otherwise>
        <step id="fallback"><script>return 0;</script></step>
        <result value="{{context.steps.fallback}}" />
      </otherwise>
    </branch>`);
    expect(validationError(emptyArm).diagnostic.code).toBe(
      'WOML_INVALID_STRUCTURE'
    );
  });

  test('requires exact typed references and points to the attribute value', () => {
    const source = validWorkflow(`
    <step id="ready"><script>return true;</script></step>
    <branch id="decision">
      <when test="Result: {{context.steps.ready}}">
        <step id="selected"><script>return 1;</script></step>
        <result value="{{context.steps.selected}}" />
      </when>
      <otherwise>
        <step id="fallback"><script>return 0;</script></step>
        <result value="{{context.steps.fallback}}" />
      </otherwise>
    </branch>`);
    const error = validationError(source);

    expect(error.diagnostic.code).toBe('WOML_INVALID_REFERENCE');
    expect(error.diagnostic.location.start.offset).toBe(
      source.indexOf('Result: {{context.steps.ready}}')
    );
  });

  test('distinguishes unknown, forward, and cross-arm references', () => {
    const unknown = validWorkflow(`
    <step id="ready"><script>return true;</script></step>
    <branch id="decision">
      <when test="{{context.steps.missing}}">
        <step id="selected"><script>return 1;</script></step>
        <result value="{{context.steps.selected}}" />
      </when>
      <otherwise>
        <step id="fallback"><script>return 0;</script></step>
        <result value="{{context.steps.fallback}}" />
      </otherwise>
    </branch>`);
    const unknownError = compileError(unknown);
    expect(unknownError.diagnostic.code).toBe('WOML_UNKNOWN_REFERENCE');
    expect(unknownError.diagnostic.location.start.offset).toBe(
      unknown.indexOf('missing')
    );

    const forward = validWorkflow(`
    <branch id="decision">
      <when test="{{context.steps.later}}">
        <step id="selected"><script>return 1;</script></step>
        <result value="{{context.steps.selected}}" />
      </when>
      <otherwise>
        <step id="fallback"><script>return 0;</script></step>
        <result value="{{context.steps.fallback}}" />
      </otherwise>
    </branch>
    <step id="later"><script>return true;</script></step>`);
    expect(compileError(forward).diagnostic.code).toBe(
      'WOML_REFERENCE_NOT_DOMINATING'
    );

    const crossArm = validWorkflow(`
    <step id="ready"><script>return true;</script></step>
    <branch id="decision">
      <when test="{{context.steps.ready}}">
        <step id="selected"><script>return 1;</script></step>
        <result value="{{context.steps.fallback}}" />
      </when>
      <otherwise>
        <step id="fallback"><script>return 0;</script></step>
        <result value="{{context.steps.fallback}}" />
      </otherwise>
    </branch>`);
    expect(compileError(crossArm).diagnostic.code).toBe(
      'WOML_REFERENCE_NOT_DOMINATING'
    );
  });

  test('uses one workflow-wide structural namespace for steps and branches', () => {
    const source = validWorkflow(`
    <step id="same"><script>return true;</script></step>
    <branch id="same">
      <when test="{{context.steps.same}}">
        <step id="selected"><script>return 1;</script></step>
        <result value="{{context.steps.selected}}" />
      </when>
      <otherwise>
        <step id="fallback"><script>return 0;</script></step>
        <result value="{{context.steps.fallback}}" />
      </otherwise>
    </branch>`);
    const error = validationError(source);

    expect(error.diagnostic.code).toBe('WOML_DUPLICATE_ID');
    expect(error.diagnostic.location.start.offset).toBe(
      source.indexOf('same', source.indexOf('<branch'))
    );
  });

  test('rejects branch tags outside their legal parent', () => {
    const source = validWorkflow(`
      <when test="{{context.trigger.ready}}">
        <step id="selected"><script>return 1;</script></step>
        <result value="{{context.steps.selected}}" />
      </when>`);

    expect(validationError(source).diagnostic.code).toBe(
      'WOML_INVALID_STRUCTURE'
    );
  });

  test('rejects missing required children and attributes', () => {
    const missingTriggers = `<workflow version="1.0.0" id="test-workflow">
  <steps><step id="a"><script>return 1;</script></step></steps>
</workflow>`;
    expect(validationError(missingTriggers).diagnostic.code).toBe(
      'WOML_TRIGGER_CONTAINER_COUNT'
    );

    const missingStepId = validWorkflow(
      '<step><script>return 1;</script></step>'
    );
    expect(validationError(missingStepId).diagnostic.code).toBe(
      'WOML_MISSING_ATTRIBUTE'
    );
  });

  test('T1 accepts multiple manual triggers and keeps canonical container order', () => {
    const multipleManual = `<workflow version="1.0.0" id="test-workflow">
  <triggers><manual id="first" /><manual id="second" /></triggers>
  <steps><step id="a"><script>return 1;</script></step></steps>
</workflow>`;
    const compiled = compile(multipleManual);
    expect(compiled.schemaVersion).toBe(7);
    expect(compiled.triggers.map(trigger => trigger.id)).toEqual([
      'first',
      'second',
    ]);

    const wrongOrder = `<workflow version="1.0.0" id="test-workflow">
  <steps><step id="a"><script>return 1;</script></step></steps>
  <triggers><manual id="start" /></triggers>
</workflow>`;
    expect(validationError(wrongOrder).diagnostic.code).toBe(
      'WOML_INVALID_STRUCTURE'
    );
  });

  test('T1 lowers the reviewed manual and webhook fixture exactly to Model v7', () => {
    const source = readFileSync(
      new URL('./fixtures/triggers-webhook.woml', import.meta.url),
      'utf8'
    );
    const expected = JSON.parse(
      readFileSync(
        new URL(
          './fixtures/triggers-webhook.compiled.v7.json',
          import.meta.url
        ),
        'utf8'
      )
    );
    const compiled = compile(source);

    expect(compiled).toEqual(expected);
    expect(compiled.schemaVersion).toBe(7);
    expect(compiled.triggers[1]).toMatchObject({
      id: 'newOrder',
      handler: 'trigger.webhook',
    });
  });

  test('T1 applies webhook defaults and explicit unauthenticated intent', () => {
    const source = `<workflow id="public-hook">
  <triggers>
    <webhook id="incoming" path="/incoming" auth="none" />
  </triggers>
  <steps><step id="capture"><script>return context.trigger;</script></step></steps>
</workflow>`;
    const compiled = compile(source);
    expect(compiled.schemaVersion).toBe(7);
    expect(compiled.triggers).toEqual([
      {
        id: 'incoming',
        handler: 'trigger.webhook',
        config: {
          kind: 'object',
          fields: {
            path: { kind: 'literal', value: '/incoming' },
            method: { kind: 'literal', value: 'POST' },
            authentication: {
              kind: 'object',
              fields: { kind: { kind: 'literal', value: 'none' } },
            },
          },
        },
      },
    ]);
  });

  test('T1 reports webhook path, method, auth, and duplicate IDs at their source', () => {
    const webhookWorkflow = (markup: string) => `<workflow id="hook-errors">
  <triggers>${markup}</triggers>
  <steps><step id="capture"><script>return 1;</script></step></steps>
</workflow>`;
    const cases = [
      {
        markup: '<webhook id="hook" path="relative" auth="none" />',
        code: 'WOML_WEBHOOK_PATH_INVALID',
        token: 'relative',
      },
      {
        markup:
          '<webhook id="hook" path="/orders" method="GET" auth="none" />',
        code: 'WOML_WEBHOOK_METHOD_UNSUPPORTED',
        token: 'GET',
      },
      {
        markup: '<webhook id="hook" path="/orders" auth="basic" />',
        code: 'WOML_WEBHOOK_AUTH_INVALID',
        token: 'basic',
      },
      {
        markup:
          '<webhook id="hook" path="/orders" auth="none" secret="{{secrets.HOOK_TOKEN}}" />',
        code: 'WOML_WEBHOOK_AUTH_INVALID',
        token: 'secret',
      },
    ];
    for (const entry of cases) {
      const source = webhookWorkflow(entry.markup);
      const error = validationError(source);
      expect(error.diagnostic.code).toBe(entry.code);
      expect(error.diagnostic.location.start.offset).toBe(
        source.indexOf(entry.token)
      );
    }

    const duplicate = webhookWorkflow(
      '<manual id="same" /><webhook id="same" path="/orders" auth="none" />'
    );
    const duplicateError = validationError(duplicate);
    expect(duplicateError.diagnostic.code).toBe(
      'WOML_TRIGGER_ID_DUPLICATE'
    );
    expect(duplicateError.diagnostic.location.start.offset).toBe(
      duplicate.lastIndexOf('same')
    );
  });

  test('T1 validates inline webhook JSON Schema without guessing bad input', () => {
    const withSchema = (schema: string) => `<workflow id="schema-hook">
  <triggers>
    <webhook id="hook" path="/orders" auth="none"><schema>${schema}</schema></webhook>
  </triggers>
  <steps><step id="capture"><script>return 1;</script></step></steps>
</workflow>`;

    const malformed = withSchema('{ "type": "object", }');
    expect(validationError(malformed).diagnostic.code).toBe(
      'WOML_WEBHOOK_SCHEMA_JSON_INVALID'
    );

    const nonObject = withSchema('["object"]');
    expect(validationError(nonObject).diagnostic.code).toBe(
      'WOML_WEBHOOK_SCHEMA_INVALID'
    );

    const invalidKeyword = withSchema('{ "type": 42 }');
    expect(validationError(invalidKeyword).diagnostic.code).toBe(
      'WOML_WEBHOOK_SCHEMA_INVALID'
    );

    const invalidPattern = withSchema(
      '{ "type": "string", "pattern": "[" }'
    );
    expect(validationError(invalidPattern).diagnostic.code).toBe(
      'WOML_WEBHOOK_SCHEMA_INVALID'
    );

    const duplicateSchema = `<workflow id="schema-hook">
  <triggers>
    <webhook id="hook" path="/orders" auth="none">
      <schema>{"type":"object"}</schema>
      <schema>{"type":"object"}</schema>
    </webhook>
  </triggers>
  <steps><step id="capture"><script>return 1;</script></step></steps>
</workflow>`;
    expect(validationError(duplicateSchema).diagnostic.code).toBe(
      'WOML_WEBHOOK_STRUCTURE_INVALID'
    );
  });

  test('T6 lowers the reviewed Slack trigger fixture exactly to Model v7', () => {
    const source = readFileSync(
      new URL('./fixtures/triggers-slack.woml', import.meta.url),
      'utf8'
    );
    const expected = JSON.parse(
      readFileSync(
        new URL('./fixtures/triggers-slack.compiled.v7.json', import.meta.url),
        'utf8'
      )
    );

    const compiled = compile(source);
    expect(compiled).toEqual(expected);
    expect(compiled.schemaVersion).toBe(7);
  });

  test('T6 accepts an omitted Slack channel filter as all visible channels', () => {
    const source = `<workflow id="slack-all-channels">
  <triggers>
    <slack id="mention" events="app-mention" bot-token="{{secrets.SLACK_BOT_TOKEN}}" app-token="{{secrets.SLACK_APP_TOKEN}}" />
  </triggers>
  <steps><step id="capture"><script>return context.trigger;</script></step></steps>
</workflow>`;
    const compiled = compile(source);
    expect(compiled.triggers[0]).toMatchObject({
      id: 'mention',
      handler: 'trigger.slack',
      config: {
        fields: {
          events: {
            items: [{ kind: 'literal', value: 'app-mention' }],
          },
          channels: { kind: 'array', items: [] },
        },
      },
    });
  });

  test('T6 reports malformed Slack events and channel filters at their source', () => {
    const slackWorkflow = (attributes: string) => `<workflow id="slack-errors">
  <triggers><slack id="message" ${attributes} bot-token="{{secrets.SLACK_BOT_TOKEN}}" app-token="{{secrets.SLACK_APP_TOKEN}}" /></triggers>
  <steps><step id="capture"><script>return 1;</script></step></steps>
</workflow>`;
    const cases = [
      {
        attributes: 'events="channel-message"',
        code: 'WOML_SLACK_TRIGGER_EVENT_INVALID',
        token: 'channel-message',
      },
      {
        attributes: 'events="app-mention,app-mention"',
        code: 'WOML_SLACK_TRIGGER_EVENT_DUPLICATE',
        token: 'app-mention',
        last: true,
      },
      {
        attributes: 'events="app-mention,"',
        code: 'WOML_SLACK_TRIGGER_LIST_INVALID',
        token: '"',
        last: true,
      },
      {
        attributes: 'events="app-mention" channels="#general"',
        code: 'WOML_SLACK_TRIGGER_CHANNEL_INVALID',
        token: '#general',
      },
      {
        attributes:
          'events="direct-message" channels="woml-testing,woml-testing"',
        code: 'WOML_SLACK_TRIGGER_CHANNEL_DUPLICATE',
        token: 'woml-testing',
        last: true,
      },
    ];
    for (const entry of cases) {
      const source = slackWorkflow(entry.attributes);
      const error = validationError(source);
      expect(error.diagnostic.code).toBe(entry.code);
      if (entry.code !== 'WOML_SLACK_TRIGGER_LIST_INVALID') {
        expect(error.diagnostic.location.start.offset).toBe(
          entry.last
            ? source.lastIndexOf(entry.token)
            : source.indexOf(entry.token)
        );
      }
    }

    const plainCredential = `<workflow id="slack-errors">
  <triggers><slack id="message" events="app-mention" bot-token="plain-text" app-token="{{secrets.SLACK_APP_TOKEN}}" /></triggers>
  <steps><step id="capture"><script>return 1;</script></step></steps>
</workflow>`;
    expect(validationError(plainCredential).diagnostic.code).toBe(
      'WOML_SECRET_LITERAL_FORBIDDEN'
    );
  });

  test('T6 keeps notification-only and trigger-only Slack attributes separate', () => {
    const source = validWorkflow(`<approval id="review">
      <notify><slack id="triggerOnly" events="app-mention" channels="#approvals" bot-token="{{secrets.SLACK_BOT_TOKEN}}" app-token="{{secrets.SLACK_APP_TOKEN}}" /></notify>
      <when-approved></when-approved><when-rejected></when-rejected>
    </approval>`);
    expect(validationError(source).diagnostic.code).toBe(
      'WOML_SLACK_UNKNOWN_ATTRIBUTE'
    );
  });

  test('T8 lowers the reviewed schedule fixture exactly to Model v7', () => {
    const source = readFileSync(
      new URL('./fixtures/triggers-schedule.woml', import.meta.url),
      'utf8'
    );
    const expected = JSON.parse(
      readFileSync(
        new URL(
          './fixtures/triggers-schedule.compiled.v7.json',
          import.meta.url
        ),
        'utf8'
      )
    );

    expect(compile(source)).toEqual(expected);
  });

  test('T8 applies UTC, skip, and cron-step defaults deterministically', () => {
    const source = `<workflow id="schedule-defaults">
  <triggers><schedule id="everySixHours" cron="0 */6 * * *" /></triggers>
  <steps><step id="capture"><script>return context.trigger;</script></step></steps>
</workflow>`;

    expect(compile(source).triggers[0]).toEqual({
      id: 'everySixHours',
      handler: 'trigger.schedule',
      config: {
        kind: 'object',
        fields: {
          cron: { kind: 'literal', value: '0 */6 * * *' },
          timezone: { kind: 'literal', value: 'UTC' },
          onMissed: { kind: 'literal', value: 'skip' },
        },
      },
    });
  });

  test('T8 reports cron, timezone, misfire, and structure errors at their source', () => {
    const scheduleWorkflow = (attributes: string, body = '') =>
      `<workflow id="schedule-errors">
  <triggers><schedule id="daily" ${attributes}>${body}</schedule></triggers>
  <steps><step id="capture"><script>return 1;</script></step></steps>
</workflow>`;
    const cases = [
      {
        attributes: 'cron="0 9 * *"',
        code: 'WOML_SCHEDULE_CRON_INVALID',
        token: '0 9 * *',
      },
      {
        attributes: 'cron="0 9 * * *" timezone="Local"',
        code: 'WOML_SCHEDULE_TIMEZONE_INVALID',
        token: 'Local',
      },
      {
        attributes: 'cron="0 9 * * *" on-missed="catch-up"',
        code: 'WOML_TRIGGER_MISFIRE_INVALID',
        token: 'catch-up',
      },
    ];
    for (const entry of cases) {
      const source = scheduleWorkflow(entry.attributes);
      const error = validationError(source);
      expect(error.diagnostic.code).toBe(entry.code);
      expect(error.diagnostic.location.start.offset).toBe(
        source.indexOf(entry.token)
      );
    }

    const nested = scheduleWorkflow(
      'cron="0 9 * * *"',
      '<manual id="nested" />'
    );
    expect(validationError(nested).diagnostic.code).toBe(
      'WOML_INVALID_STRUCTURE'
    );
  });

  test('keeps interval and event runtime-staged after T8', () => {
    const fixtureNames = [
      'triggers-interval.woml',
      'triggers-event.woml',
    ];
    for (const name of fixtureNames) {
      const source = readFileSync(
        new URL(`./fixtures/${name}`, import.meta.url),
        'utf8'
      );
      expect(validationError(source).diagnostic.code).toBe(
        'WOML_FEATURE_NOT_EXECUTABLE'
      );
    }
  });

  test('requires the workflow root and validates trigger IDs', () => {
    const wrongRoot =
      '<steps><step id="a"><script>return 1;</script></step></steps>';
    expect(validationError(wrongRoot).diagnostic.code).toBe(
      'WOML_EXPECTED_WORKFLOW_ROOT'
    );

    const invalidTrigger = validWorkflow().replace(
      'id="start"',
      'id="bad-trigger"'
    );
    expect(validationError(invalidTrigger).diagnostic.code).toBe(
      'WOML_INVALID_ID'
    );
  });

  test('rejects multiple step operations', () => {
    const source = validWorkflow(
      '<step id="a"><script>return 1;</script><script>return 2;</script></step>'
    );
    const error = validationError(source);

    expect(error.diagnostic.code).toBe('WOML_STEP_OPERATION_COUNT');
    expect(error.diagnostic.location.start.offset).toBe(
      source.lastIndexOf('<script>')
    );
  });

  test('lowers the workflow version as metadata and rejects empty metadata', () => {
    expect(compile(validWorkflow()).metadata?.version).toBe('1.0.0');

    const emptyName = validWorkflow(
      '<step id="a" name="   "><script>return 1;</script></step>'
    );
    expect(validationError(emptyName).diagnostic.code).toBe(
      'WOML_EMPTY_METADATA'
    );

    const emptyVersion = validWorkflow().replace(
      'version="1.0.0"',
      'version="   "'
    );
    expect(validationError(emptyVersion).diagnostic.code).toBe(
      'WOML_EMPTY_METADATA'
    );
  });

  test('does not require a workflow version', () => {
    const source = validWorkflow().replace(' version="1.0.0"', '');

    expect(compile(source).metadata).toBeUndefined();
  });

  test('rejects woml-version because version belongs to the workflow', () => {
    const source = validWorkflow().replace(
      'version="1.0.0"',
      'woml-version="0.1"'
    );

    expect(validationError(source).diagnostic.code).toBe(
      'WOML_UNKNOWN_ATTRIBUTE'
    );
  });
});

describe('inspectCompiledWorkflowGraph', () => {
  test('reports graph-boundary failures without knowing WOML syntax', () => {
    const graph: CompiledWorkflowGraph = {
      entryNodeIds: ['a'],
      nodes: [
        {
          id: 'a',
          handler: 'runtime.script',
          inputs: { kind: 'literal', value: null },
        },
        {
          id: 'b',
          handler: 'runtime.script',
          inputs: { kind: 'literal', value: null },
        },
        {
          id: 'c',
          handler: 'runtime.script',
          inputs: { kind: 'literal', value: null },
        },
      ],
      edges: [
        { id: 'cycle', from: 'a', to: 'b', condition: { kind: 'always' } },
        { id: 'cycle', from: 'b', to: 'a', condition: { kind: 'always' } },
        {
          id: 'missing',
          from: 'b',
          to: 'missing',
          condition: { kind: 'always' },
        },
      ],
    };

    const codes = inspectCompiledWorkflowGraph(graph).map(issue => issue.code);
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
        {
          id: 'a',
          handler: 'runtime.script',
          inputs: { kind: 'literal', value: null },
        },
        {
          id: 'b',
          handler: 'runtime.script',
          inputs: { kind: 'literal', value: null },
        },
      ],
      edges: [],
    };

    expect(
      inspectCompiledWorkflowGraph(graph, { requireSingleTerminal: true }).map(
        issue => issue.code
      )
    ).toContain('TERMINAL_NODE_COUNT');
  });

  test('rejects malformed branch selector, group, and result contracts', () => {
    const source = readFileSync(
      new URL('./fixtures/branch.woml', import.meta.url),
      'utf8'
    );
    const compiled = compile(source);

    const badSelector = {
      ...compiled.graph,
      nodes: compiled.graph.nodes.map((node, index) =>
        index === 1 ? { ...node, id: '__woml_branch__wrong__select' } : node
      ),
    };
    expect(
      inspectCompiledWorkflowGraph(badSelector).map(issue => issue.code)
    ).toContain('INVALID_BRANCH_SELECTOR');

    const badGroup = {
      ...compiled.graph,
      edges: compiled.graph.edges.map((edge, index) =>
        index === 1 ? { ...edge, id: 'decision:when:1' } : edge
      ),
    };
    expect(
      inspectCompiledWorkflowGraph(badGroup).map(issue => issue.code)
    ).toContain('INVALID_BRANCH_GROUP');

    const badResult = {
      ...compiled.graph,
      nodes: compiled.graph.nodes.map(node =>
        node.id === 'decision'
          ? { ...node, inputs: { kind: 'object' as const, fields: {} } }
          : node
      ),
    };
    expect(
      inspectCompiledWorkflowGraph(badResult).map(issue => issue.code)
    ).toContain('INVALID_BRANCH_RESULT');

    const badJoin = {
      ...compiled.graph,
      edges: compiled.graph.edges.filter(
        edge => edge.id !== 'acceptContent-to-decision'
      ),
    };
    expect(
      inspectCompiledWorkflowGraph(badJoin).map(issue => issue.code)
    ).toContain('INVALID_BRANCH_GROUP');
  });

  test('rejects malformed parallel start, ownership, route, and join contracts', () => {
    const source = readFileSync(
      new URL('./fixtures/parallel.woml', import.meta.url),
      'utf8'
    );
    const compiled = compile(source);

    const badStart = {
      ...compiled.graph,
      nodes: compiled.graph.nodes.map(node =>
        node.handler === 'engine.parallel-start'
          ? {
              ...node,
              inputs: {
                kind: 'object' as const,
                fields: {
                  concurrency: { kind: 'literal' as const, value: 3 },
                  onError: { kind: 'literal' as const, value: 'continue' },
                },
              },
            }
          : node
      ),
    };
    expect(
      inspectCompiledWorkflowGraph(badStart).map(issue => issue.code)
    ).toContain('INVALID_PARALLEL_GROUP');

    const missingOwner = {
      ...compiled.graph,
      edges: compiled.graph.edges.map(edge =>
        edge.id === 'fieldData:child:0'
          ? { ...edge, parallelId: undefined }
          : edge
      ),
    };
    expect(
      inspectCompiledWorkflowGraph(missingOwner).map(issue => issue.code)
    ).toContain('INVALID_PARALLEL_GROUP');

    const bypassedJoin = {
      ...compiled.graph,
      edges: [
        ...compiled.graph.edges,
        {
          id: 'loadWeather-to-buildReport',
          from: 'loadWeather',
          to: 'buildReport',
          condition: { kind: 'always' as const },
        },
      ],
    };
    expect(
      inspectCompiledWorkflowGraph(bypassedJoin).map(issue => issue.code)
    ).toContain('INVALID_PARALLEL_GROUP');

    const terminalJoin = {
      ...compiled.graph,
      edges: compiled.graph.edges.filter(
        edge => edge.id !== 'fieldData-to-buildReport'
      ),
    };
    expect(
      inspectCompiledWorkflowGraph(terminalJoin).map(issue => issue.code)
    ).toContain('INVALID_PARALLEL_GROUP');
  });

  test('rejects malformed approval wait, ownership, decision, and join contracts', () => {
    const source = readFileSync(
      new URL('./fixtures/approval.woml', import.meta.url),
      'utf8'
    );
    const compiled = compile(source);

    const badWait = {
      ...compiled.graph,
      nodes: compiled.graph.nodes.map(node =>
        node.handler === 'engine.approval-wait'
          ? {
              ...node,
              inputs: {
                kind: 'object' as const,
                fields: {
                  onTimeout: { kind: 'literal' as const, value: 'approve' },
                },
              },
            }
          : node
      ),
    };
    expect(
      inspectCompiledWorkflowGraph(badWait).map(issue => issue.code)
    ).toContain('INVALID_APPROVAL_GROUP');

    const missingOwner = {
      ...compiled.graph,
      edges: compiled.graph.edges.map(edge =>
        edge.id === 'editorApproval:approved'
          ? { ...edge, approvalId: undefined }
          : edge
      ),
    };
    expect(
      inspectCompiledWorkflowGraph(missingOwner).map(issue => issue.code)
    ).toContain('INVALID_APPROVAL_GROUP');

    const wrongDecision = {
      ...compiled.graph,
      edges: compiled.graph.edges.map(edge =>
        edge.id === 'editorApproval:approved'
          ? {
              ...edge,
              condition: {
                kind: 'equals' as const,
                left: {
                  kind: 'contextReference' as const,
                  path: ['steps', 'editorApproval', 'decision'],
                },
                right: { kind: 'literal' as const, value: 'rejected' },
              },
            }
          : edge
      ),
    };
    expect(
      inspectCompiledWorkflowGraph(wrongDecision).map(issue => issue.code)
    ).toContain('INVALID_APPROVAL_GROUP');

    const badJoin = {
      ...compiled.graph,
      nodes: compiled.graph.nodes.map(node =>
        node.handler === 'engine.approval-join'
          ? { ...node, id: '__woml_approval__wrong__join' }
          : node
      ),
    };
    expect(
      inspectCompiledWorkflowGraph(badJoin).map(issue => issue.code)
    ).toContain('INVALID_APPROVAL_GROUP');
  });
});
