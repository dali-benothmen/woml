import { afterAll, beforeAll, describe, expect, test } from 'bun:test';
import { cp, mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';

const sourceRoot = resolve(
  import.meta.dir,
  '../../woml/tests/fixtures/reusable-definitions'
);
let fixtureRoot = '';

beforeAll(async () => {
  fixtureRoot = await mkdtemp(resolve(tmpdir(), 'woml-reusable-definitions-'));
  await cp(sourceRoot, fixtureRoot, { recursive: true });
});

afterAll(async () => {
  await rm(fixtureRoot, { recursive: true, force: true });
});

async function invoke(args: readonly string[]) {
  let stdout = '';
  let stderr = '';
  const io: CliIo = {
    stdout: value => { stdout += value; },
    stderr: value => { stderr += value; },
  };
  const exitCode = await runCli(args, io);
  return { exitCode, stdout, stderr };
}

describe('reusable definition CLI authoring', () => {
  test('checks a reusable step and refreshes custom-tag editor metadata', async () => {
    const path = resolve(fixtureRoot, 'calculate-discount.woml');
    const result = await invoke(['check', path]);

    expect(result.exitCode).toBe(0);
    expect(result.stderr).toBe('');
    expect(result.stdout).toContain('reusable step definition');
    expect(result.stdout).toContain('not independently runnable');
    const editorData = JSON.parse(
      await readFile(resolve(fixtureRoot, 'woml-custom-data.json'), 'utf8')
    );
    expect(editorData.tags).toEqual([]);
  });

  test('checks a workflow dependency graph and emits imported tag completions', async () => {
    const result = await invoke([
      'check',
      resolve(fixtureRoot, 'workflow.woml'),
    ]);

    expect(result.exitCode).toBe(0);
    expect(result.stderr).toBe('');
    expect(result.stdout).toContain('Definitions: 2; pinned sources: 5.');
    expect(result.stdout).toContain('<calculate-discount>');
    expect(result.stdout).toContain('<telegram>');
    const editorData = JSON.parse(
      await readFile(resolve(fixtureRoot, 'woml-custom-data.json'), 'utf8')
    );
    expect(editorData.tags.map((tag: { name: string }) => tag.name)).toEqual([
      'calculate-discount',
      'telegram',
    ]);
    expect(
      editorData.tags.find((tag: { name: string }) => tag.name === 'telegram')
        .attributes
    ).toEqual(expect.arrayContaining([
      expect.objectContaining({ name: 'bot-token' }),
      expect.objectContaining({ name: 'chat-id' }),
    ]));
  });

  test('check compiles a runtime-ready custom-step workflow', async () => {
    const path = resolve(fixtureRoot, 'custom-step-workflow.woml');
    const checked = await invoke(['check', path, '--json']);
    expect(checked.exitCode).toBe(0);
    expect(checked.stderr).toBe('');
    const definitionPackage = JSON.parse(checked.stdout);
    expect(definitionPackage).toMatchObject({
      schemaVersion: 9,
      profile: 'woml.definition-package/v9',
      runtimeReady: true,
      workflow: { model: { schemaVersion: 14 } },
    });
  });

  test('check compiles provider delivery and definition-owned lifecycle artifacts', async () => {
    const path = resolve(fixtureRoot, 'custom-provider-workflow.woml');
    const checked = await invoke(['check', path, '--json']);
    expect(checked.exitCode).toBe(0);
    expect(checked.stderr).toBe('');
    const definitionPackage = JSON.parse(checked.stdout);
    expect(definitionPackage).toMatchObject({
      schemaVersion: 9,
      runtimeReady: true,
      definitions: [
        expect.objectContaining({
          alias: 'telegram',
          kind: 'notification-provider',
        }),
      ],
    });
    expect(
      definitionPackage.workflow.model.reusableDefinitions.filter(
        (item: { kind: string }) => item.kind === 'notification-provider'
      )
    ).toHaveLength(2);
    expect(
      definitionPackage.artifacts.filter((item: { path: string }) =>
        item.path.includes('.on-')
      )
    ).toHaveLength(4);
  });

  test('package identity is stable across different directories and timestamps', async () => {
    const otherRoot = await mkdtemp(resolve(tmpdir(), 'woml-reusable-copy-'));
    try {
      await cp(sourceRoot, otherRoot, { recursive: true });
      const first = await invoke([
        'check',
        resolve(fixtureRoot, 'custom-step-workflow.woml'),
        '--json',
      ]);
      const second = await invoke([
        'check',
        resolve(otherRoot, 'custom-step-workflow.woml'),
        '--json',
      ]);
      expect(first.exitCode).toBe(0);
      expect(second.exitCode).toBe(0);
      expect(JSON.parse(second.stdout)).toEqual(JSON.parse(first.stdout));
    } finally {
      await rm(otherRoot, { recursive: true, force: true });
    }
  });

  test('explains direct definition runs and accepts provider lifecycle compilation', async () => {
    const direct = await invoke([
      'run',
      resolve(fixtureRoot, 'calculate-discount.woml'),
    ]);
    expect(direct.exitCode).toBe(1);
    expect(direct.stderr).toContain('WOML_DEFINITION_NOT_RUNNABLE');

    await rm(resolve(fixtureRoot, 'woml-custom-data.json'), { force: true });
    const workflow = await invoke([
      'check',
      resolve(fixtureRoot, 'workflow.woml'),
    ]);
    expect(workflow.exitCode).toBe(0);
    expect(workflow.stdout).toContain('Execution: custom steps and notification providers are runnable.');
    const editorData = JSON.parse(
      await readFile(resolve(fixtureRoot, 'woml-custom-data.json'), 'utf8')
    );
    expect(editorData.tags).toHaveLength(2);
  });

  test('folder discovery ignores reusable files but requires a runnable workflow', async () => {
    const definitionsOnly = await mkdtemp(
      resolve(tmpdir(), 'woml-definitions-only-')
    );
    try {
      await cp(
        resolve(fixtureRoot, 'calculate-discount.woml'),
        resolve(definitionsOnly, 'calculate-discount.woml')
      );
      await cp(
        resolve(fixtureRoot, 'pricing.ts'),
        resolve(definitionsOnly, 'pricing.ts')
      );
      await cp(
        resolve(fixtureRoot, 'pricing-helper.ts'),
        resolve(definitionsOnly, 'pricing-helper.ts')
      );
      const result = await invoke(['run', definitionsOnly]);
      expect(result.exitCode).toBe(1);
      expect(result.stderr).toContain('WOML_RUNNABLE_WORKFLOW_REQUIRED');
    } finally {
      await rm(definitionsOnly, { recursive: true, force: true });
    }
  });
});
