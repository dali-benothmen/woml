import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

import {
  compileWoml,
  createRuntimeHandlerRegistry,
  executeWorkflow,
  HandlerRegistry,
  parseWoml,
  WorkflowExecutionError,
  type CompiledWorkflowDefinition,
  type JsonValue,
  type WorkflowContext,
} from '../src';

function compile(source: string): CompiledWorkflowDefinition {
  return compileWoml(parseWoml(source, { file: 'workflow.woml' }));
}

function oneStepWorkflow(script: string): CompiledWorkflowDefinition {
  return compile(`<workflow woml-version="0.1" id="runtime-test">
  <triggers><manual id="start" /></triggers>
  <steps><step id="a"><script>${script}</script></step></steps>
</workflow>`);
}

async function executionError(
  promise: Promise<unknown>,
): Promise<WorkflowExecutionError> {
  try {
    await promise;
  } catch (error) {
    if (error instanceof WorkflowExecutionError) return error;
    throw error;
  }
  throw new Error('Expected workflow execution to fail.');
}

describe('executeWorkflow', () => {
  test('executes hello.woml and threads context between both script workers', async () => {
    const source = readFileSync(new URL('./fixtures/hello.woml', import.meta.url), 'utf8');
    const expectedContexts = JSON.parse(
      readFileSync(
        new URL('./fixtures/hello.context.v0.1.json', import.meta.url),
        'utf8',
      ),
    );
    const workflow = compileWoml(parseWoml(source, { file: 'hello.woml' }));

    const execution = await executeWorkflow(workflow);

    expect(execution.workflowId).toBe('hello');
    expect(execution.executionOrder).toEqual(['a', 'b']);
    expect(execution.terminalNodeId).toBe('b');
    expect(execution.result).toEqual({ message: 'Hello World' });
    expect(execution.context).toEqual(expectedContexts.afterB);
    expect(Object.isFrozen(execution.context)).toBe(true);
    expect(Object.isFrozen(execution.context.steps)).toBe(true);
  });

  test('supports trigger data and top-level await', async () => {
    const workflow = oneStepWorkflow(`
      await Promise.resolve();
      return { greeting: \`Hello \${context.trigger.name}\` };
    `);

    const execution = await executeWorkflow(workflow, {
      trigger: { name: 'Ada' },
    });

    expect(execution.result).toEqual({ greeting: 'Hello Ada' });
    expect(execution.context.trigger).toEqual({ name: 'Ada' });
  });

  test('provides context only, without context.run or services', async () => {
    const workflow = oneStepWorkflow(`
      return {
        hasRun: "run" in context,
        hasServices: typeof services !== "undefined"
      };
    `);

    const execution = await executeWorkflow(workflow);
    expect(execution.result).toEqual({ hasRun: false, hasServices: false });
  });

  test('uses a fresh worker scope for every script node', async () => {
    const workflow = compile(`<workflow woml-version="0.1" id="worker-isolation">
  <triggers><manual id="start" /></triggers>
  <steps>
    <step id="a"><script>
      globalThis.__womlLeakedState = "from-a";
      return { set: true };
    </script></step>
    <step id="b"><script>
      return { leaked: globalThis.__womlLeakedState ?? null };
    </script></step>
  </steps>
</workflow>`);

    const execution = await executeWorkflow(workflow);
    expect(execution.result).toEqual({ leaked: null });
  });

  test('deep-freezes context inside the worker', async () => {
    const workflow = oneStepWorkflow(`
      context.trigger.name = "Changed";
      return { name: context.trigger.name };
    `);
    const trigger = { name: 'Original' };

    const error = await executionError(executeWorkflow(workflow, { trigger }));

    expect(error.code).toBe('WOML_SCRIPT_FAILED');
    expect(error.nodeId).toBe('a');
    expect(trigger).toEqual({ name: 'Original' });
  });

  test('reports script exceptions with the failing node ID', async () => {
    const workflow = oneStepWorkflow('throw new Error("boom");');
    const error = await executionError(executeWorkflow(workflow));

    expect(error.code).toBe('WOML_SCRIPT_FAILED');
    expect(error.nodeId).toBe('a');
    expect(error.message).toContain('boom');
  });

  test.each([
    ['undefined', 'return undefined;'],
    ['BigInt', 'return 1n;'],
    ['function', 'return { callback() {} };'],
    [
      'circular object',
      'const value = {}; value.self = value; return value;',
    ],
  ])('rejects a non-JSON %s result', async (_label, script) => {
    const error = await executionError(
      executeWorkflow(oneStepWorkflow(script)),
    );

    expect(error.code).toBe('WOML_NON_JSON_RESULT');
    expect(error.nodeId).toBe('a');
  });

  test('rejects unknown handlers before invoking a node', async () => {
    const workflow = structuredClone(oneStepWorkflow('return { ok: true };'));
    (workflow.graph.nodes[0] as { handler: string }).handler = 'missing.handler';

    const error = await executionError(executeWorkflow(workflow));
    expect(error.code).toBe('WOML_UNKNOWN_HANDLER');
    expect(error.nodeId).toBe('a');
  });

  test('rejects an invalid compiled graph before execution', async () => {
    const workflow = structuredClone(oneStepWorkflow('return { ok: true };'));
    (workflow.graph.entryNodeIds as string[]).push('missing');

    const error = await executionError(executeWorkflow(workflow));
    expect(error.code).toBe('WOML_INVALID_DAG');
  });

  test('does not execute staged contextReference inputs', async () => {
    const workflow = structuredClone(oneStepWorkflow('return { ok: true };'));
    (workflow.graph.nodes[0] as { inputs: unknown }).inputs = {
      kind: 'contextReference',
      path: ['trigger'],
    };

    const error = await executionError(executeWorkflow(workflow));
    expect(error.code).toBe('WOML_UNSUPPORTED_INPUT_EXPRESSION');
    expect(error.nodeId).toBe('a');
  });

  test('publishes only successful outputs to following contexts', async () => {
    const workflow = compile(`<workflow woml-version="0.1" id="publication-test">
  <triggers><manual id="start" /></triggers>
  <steps>
    <step id="a"><script>return { ignored: true };</script></step>
    <step id="b"><script>return { ignored: true };</script></step>
  </steps>
</workflow>`);
    const observedContexts: WorkflowContext[] = [];
    const registry = new HandlerRegistry();
    registry.register('runtime.script', async ({ node, context }) => {
      observedContexts.push(context);
      if (node.id === 'a') return { x: 'World' };
      const circular: Record<string, unknown> = {};
      circular.self = circular;
      return circular as unknown as JsonValue;
    });

    const error = await executionError(executeWorkflow(workflow, { registry }));

    expect(error.code).toBe('WOML_NON_JSON_RESULT');
    expect(observedContexts).toHaveLength(2);
    expect(observedContexts[1].steps).toEqual({ a: { x: 'World' } });
    expect('b' in observedContexts[1].steps).toBe(false);
  });

  test('the default registry contains only runtime.script', () => {
    const registry = createRuntimeHandlerRegistry();
    expect(registry.size).toBe(1);
    expect(registry.handlerIds).toEqual(['runtime.script']);
  });
});
