import { describe, expect, test } from 'bun:test';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';
import { createSecretStore } from '../src/secrets';
import {
  executeWorkflowWithRustDurable,
  resumeWorkflowWithRustDurable,
} from '../src/rust-executor';
import { compileWoml, parseWoml } from 'woml';

const example = resolve(import.meta.dir, '../../examples/switchWorkflow.woml');
const nativeCorePath = resolve(
  import.meta.dir,
  `../dist/woml-core.${process.platform}-${process.arch}.node`
);
const scriptHostPath = resolve(import.meta.dir, '../dist/script-host.js');

async function invoke(args: readonly string[]) {
  let stdout = '';
  let stderr = '';
  const io: CliIo = {
    stdout: value => {
      stdout += value;
    },
    stderr: value => {
      stderr += value;
    },
  };
  const exitCode = await runCli(args, io, {
    createSecretStore: () => createSecretStore(),
    readSecret: async () => '',
    nativeCorePath,
  });
  return { exitCode, stdout, stderr };
}

function source(selector: string): string {
  return `<woml>
    <workflow id="switch-cli" version="1.0.0">
      <triggers><manual id="start" /></triggers>
      <steps>
        <step id="load"><script>return { provider: ${selector} };</script></step>
        <switch id="delivery" value="{{context.steps.load.provider}}">
          <case value="slack">
            <step id="slack"><script>return { route: "slack" };</script></step>
            <result value="{{context.steps.slack}}" />
          </case>
          <case value="email">
            <step id="email"><script>throw new Error("unselected route ran");</script></step>
            <result value="{{context.steps.email}}" />
          </case>
          <default>
            <step id="fallback"><script>return { route: "fallback" };</script></step>
            <result value="{{context.steps.fallback}}" />
          </default>
        </switch>
        <step id="finish"><script>return context.steps.delivery;</script></step>
      </steps>
    </workflow>
  </woml>`;
}

describe('switch CLI execution', () => {
  test('runs the exact selected route and publishes its stable result', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-switch-exact-'));
    try {
      const checked = await invoke(['check', example]);
      expect(checked.exitCode).toBe(0);
      expect(checked.stdout).toContain(
        'Model v14 exact-string switch routing and merged results are executable'
      );

      const executed = await invoke([
        'test',
        example,
        '--state',
        join(directory, 'state.sqlite'),
      ]);
      expect(executed.exitCode, executed.stderr).toBe(0);
      expect(JSON.parse(executed.stdout)).toEqual({
        message: 'Order order-42 delivered with slack',
        delivered: true,
      });
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  test('uses the default route and never executes an unselected case', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-switch-default-'));
    const workflowPath = join(directory, 'default.woml');
    await writeFile(workflowPath, source('"sms"'));
    try {
      const executed = await invoke([
        'test',
        workflowPath,
        '--state',
        join(directory, 'state.sqlite'),
      ]);
      expect(executed.exitCode, executed.stderr).toBe(0);
      expect(JSON.parse(executed.stdout)).toEqual({ route: 'fallback' });
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  test('fails durably with an actionable code when value is not a string', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-switch-invalid-'));
    const workflowPath = join(directory, 'invalid.woml');
    await writeFile(workflowPath, source('{ nested: true }'));
    try {
      const executed = await invoke([
        'test',
        workflowPath,
        '--state',
        join(directory, 'state.sqlite'),
      ]);
      expect(executed.exitCode).toBe(1);
      expect(executed.stderr).toContain('WOML_SWITCH_VALUE_INVALID');
      expect(executed.stderr).toContain('must resolve to a JSON string');
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  test('recovery reuses the durable selected case without selecting again', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-switch-recovery-'));
    const statePath = join(directory, 'state.sqlite');
    try {
      const workflow = compileWoml(
        parseWoml(source('"slack"'), { file: 'recovery.woml' })
      );
      const first = await executeWorkflowWithRustDurable(workflow, statePath, {
        nativeCorePath,
        scriptHostPath,
      });
      const recovered = await resumeWorkflowWithRustDurable(
        workflow,
        statePath,
        first.runId,
        { nativeCorePath, scriptHostPath }
      );
      expect(recovered.result).toEqual({ route: 'slack' });
      expect(
        recovered.events.filter(event => event.type === 'choice_selected')
      ).toHaveLength(1);
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  test('composes a result-producing switch inside a durable fork branch', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-switch-fork-'));
    const workflowPath = join(directory, 'switch-in-fork.woml');
    await writeFile(
      workflowPath,
      `<woml>
        <workflow id="switch-fork" version="1.0.0">
          <triggers><manual id="start" /></triggers>
          <steps>
            <step id="prepare"><script>return { provider: "slack" };</script></step>
            <fork id="distribution" join="all">
              <branch id="deliveryRoute">
                <switch id="delivery" value="{{context.steps.prepare.provider}}">
                  <case value="slack">
                    <step id="slack"><script>return { route: "slack" };</script></step>
                    <result value="{{context.steps.slack}}" />
                  </case>
                  <default>
                    <step id="fallback"><script>return { route: "fallback" };</script></step>
                    <result value="{{context.steps.fallback}}" />
                  </default>
                </switch>
              </branch>
              <branch id="archiveRoute">
                <step id="archive"><script>return { stored: true };</script></step>
              </branch>
            </fork>
            <step id="finish">
              <script>
                return {
                  route: context.steps.delivery.route,
                  archived: context.steps.archive.stored
                };
              </script>
            </step>
          </steps>
        </workflow>
      </woml>`
    );
    try {
      const executed = await invoke([
        'test',
        workflowPath,
        '--state',
        join(directory, 'state.sqlite'),
      ]);
      expect(executed.exitCode, executed.stderr).toBe(0);
      expect(JSON.parse(executed.stdout)).toEqual({
        route: 'slack',
        archived: true,
      });
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });
});
