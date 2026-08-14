import { expect, test } from 'bun:test';
import { existsSync } from 'node:fs';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { compileWoml, parseWoml } from 'woml';

import {
  executeWorkflowWithRustDurable,
  inspectRunPresentationWithRust,
  listRunPresentationsWithRust,
} from '../src/rust-executor';

const packageRoot = resolve(import.meta.dir, '..');
const nativeCorePath = resolve(
  packageRoot,
  'dist',
  `woml-core.${process.platform}-${process.arch}.node`
);
const nativeTest = existsSync(nativeCorePath) ? test : test.skip;

nativeTest('the native boundary returns strict durable Run Presentation v1 snapshots', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'woml-run-presentation-'));
  const statePath = join(directory, 'state.sqlite');
  const sourcePath = resolve(packageRoot, '../woml/tests/fixtures/hello.woml');
  try {
    const workflow = compileWoml(
      parseWoml(await Bun.file(sourcePath).text(), { file: sourcePath })
    );
    const execution = await executeWorkflowWithRustDurable(workflow, statePath, {
      nativeCorePath,
      scriptHostPath: resolve(packageRoot, 'src/script-host.ts'),
    });
    const presentation = inspectRunPresentationWithRust(
      statePath,
      execution.runId,
      { nativeCorePath }
    );
    expect(presentation).toMatchObject({
      profile: 'woml.run-presentation/v1',
      runId: execution.runId,
      status: 'succeeded',
      result: { message: 'Hello World' },
      workflow: { id: 'hello', name: 'Hello WOML' },
      summary: { succeeded: 2, failed: 0, total: 2 },
    });
    expect(presentation.steps.map(step => step.id)).toEqual(['a', 'b']);

    const recent = listRunPresentationsWithRust(statePath, 'hello', 10, {
      nativeCorePath,
    });
    expect(recent.profile).toBe('woml.run-presentation-list/v1');
    expect(recent.runs.map(run => run.runId)).toEqual([execution.runId]);

    const lifecycleSource = resolve(packageRoot, '../examples/lifecycleWorkflow.woml');
    const lifecycleWorkflow = compileWoml(
      parseWoml(await Bun.file(lifecycleSource).text(), { file: lifecycleSource })
    );
    const lifecycleExecution = await executeWorkflowWithRustDurable(
      lifecycleWorkflow,
      join(directory, 'lifecycle.sqlite'),
      { nativeCorePath, scriptHostPath: resolve(packageRoot, 'src/script-host.ts') }
    );
    const lifecyclePresentation = inspectRunPresentationWithRust(
      join(directory, 'lifecycle.sqlite'),
      lifecycleExecution.runId,
      { nativeCorePath }
    );
    expect(lifecyclePresentation.lifecycle.map(item => item.hook)).toEqual([
      'on-start',
      'on-step-start',
      'on-step-success',
      'on-step-start',
      'on-step-success',
      'on-success',
      'on-step-complete',
      'on-step-complete',
      'on-complete',
    ]);
    expect(lifecyclePresentation.lifecycle.every(item => item.status === 'succeeded')).toBe(true);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
