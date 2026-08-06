import { describe, expect, test } from 'bun:test';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import {
  compileWoml,
  executeWorkflow,
  parseWoml,
  type CompiledWorkflowDefinition,
} from 'woml';
import {
  compiledDefinitionHash,
  executeWorkflowWithRustDurable,
  executeWorkflowWithRust,
  recoverDurableRuns,
  RustWorkflowExecutionError,
} from '../src/rust-executor';

const nativeCorePath = process.env.WOML_RUST_CORE_PATH;
const packageRoot = resolve(import.meta.dir, '..');
const scriptHostPath = resolve(packageRoot, 'src/script-host.ts');
const crashingHostPath = resolve(
  packageRoot,
  'tests/fixtures/crashing-script-host.ts',
);
const nativeTest = nativeCorePath === undefined ? test.skip : test;

async function helloWorkflow(): Promise<CompiledWorkflowDefinition> {
  const path = resolve(packageRoot, '../woml/tests/fixtures/hello.woml');
  return compileWoml(parseWoml(await Bun.file(path).text(), { file: path }));
}

function replaceFirstScript(
  workflow: CompiledWorkflowDefinition,
  source: string,
): CompiledWorkflowDefinition {
  const mutable = structuredClone(workflow) as unknown as {
    graph: {
      nodes: Array<{
        inputs: {
          fields: Record<string, { value: unknown }>;
        };
      }>;
    };
  };
  mutable.graph.nodes[0].inputs.fields.source.value = source;
  return mutable as unknown as CompiledWorkflowDefinition;
}

async function rustError(
  workflow: CompiledWorkflowDefinition,
  options: { readonly scriptTimeoutMs?: number; readonly scriptHostPath?: string } = {},
): Promise<string> {
  try {
    await executeWorkflowWithRust(workflow, {
      nativeCorePath,
      scriptHostPath: options.scriptHostPath ?? scriptHostPath,
      scriptTimeoutMs: options.scriptTimeoutMs,
    });
    throw new Error('Expected Rust execution to fail.');
  } catch (error) {
    if (error instanceof RustWorkflowExecutionError) {
      return `${error.code}: ${error.message}`;
    }
    return error instanceof Error ? error.message : String(error);
  }
}

describe('Rust to Bun workflow execution', () => {
  test('pins the production definition hash for hello.woml', async () => {
    expect(compiledDefinitionHash(await helloWorkflow())).toBe(
      'sha256:74d4a6799119042d1cdcf2ed3e1e8e30228b3fbb80ad6750c1256ebd335b03ae',
    );
  });

  nativeTest('matches the TypeScript reference result, context, and order', async () => {
    const workflow = await helloWorkflow();
    const reference = await executeWorkflow(workflow);
    const rust = await executeWorkflowWithRust(workflow, {
      nativeCorePath,
      scriptHostPath,
    });

    expect(rust.result).toEqual(reference.result);
    expect(rust.context).toEqual(reference.context);
    expect(rust.executionOrder).toEqual(reference.executionOrder);
    expect(rust.executionOrder).toEqual(['a', 'b']);
    expect(rust.events.map((event) => event.type)).toEqual([
      'run_started',
      'step_attempt_started',
      'step_attempt_succeeded',
      'step_attempt_started',
      'step_attempt_succeeded',
      'run_succeeded',
    ]);
  });

  nativeTest('preserves script throw, timeout, invalid result, and host crash', async () => {
    const workflow = await helloWorkflow();

    expect(
      await rustError(replaceFirstScript(workflow, 'throw new Error("boom");')),
    ).toContain('WOML_SCRIPT_THROWN');
    expect(
      await rustError(replaceFirstScript(workflow, 'while (true) {}'), {
        scriptTimeoutMs: 40,
      }),
    ).toContain('WOML_SCRIPT_TIMEOUT');
    expect(
      await rustError(replaceFirstScript(workflow, 'return undefined;')),
    ).toContain('WOML_SCRIPT_NON_JSON_RESULT');
    expect(
      await rustError(workflow, { scriptHostPath: crashingHostPath }),
    ).toContain('WOML_SCRIPT_HOST_CRASHED');
  });

  nativeTest('persists a completed run and reconstructs it through the native recovery API', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-r4-native-'));
    const database = join(directory, 'runs.sqlite');
    try {
      const workflow = await helloWorkflow();
      const execution = await executeWorkflowWithRustDurable(workflow, database, {
        nativeCorePath,
        scriptHostPath,
      });
      expect(execution.result).toEqual({ message: 'Hello World' });
      expect(execution.events).toHaveLength(6);

      const recovery = recoverDurableRuns(database, { nativeCorePath });
      expect(recovery).toEqual({
        inspectedRuns: 1,
        recoveredRuns: 0,
        interruptedAttempts: 0,
        resumableRuns: 0,
      });
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });
});
