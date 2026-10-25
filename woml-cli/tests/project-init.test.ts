import { afterEach, describe, expect, test } from 'bun:test';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import {
  initializeWomlProject,
  WomlProjectInitializationError,
} from '../src/project-init';
import { resolveWomlProject } from '../src/project';

const temporary: string[] = [];

afterEach(async () => {
  await Promise.all(
    temporary.splice(0).map(path => rm(path, { recursive: true, force: true }))
  );
});

describe('WOML project initialization', () => {
  test('creates only the reviewed minimal local project and resolves it', async () => {
    const parent = await mkdtemp(join(tmpdir(), 'woml-init-'));
    temporary.push(parent);
    const target = join(parent, 'Payments Operations');
    const initialized = await initializeWomlProject(target);
    const project = await resolveWomlProject(initialized.manifestPath);
    const manifest = await readFile(initialized.manifestPath, 'utf8');

    expect(project.name).toBe('payments-operations');
    expect(project.workflows).toHaveLength(1);
    expect(manifest).not.toMatch(/token|secret|credential|privateKey/iu);
    expect(await readFile(initialized.workflowPath, 'utf8')).toContain(
      '<workflow id="hello" name="Hello WOML">'
    );
    const guide = await readFile(initialized.authoringGuidePath, 'utf8');
    expect(guide).toContain('woml check --project workflow.json');
    expect(guide).toContain('Do not weaken, replace, inline,');
    expect(guide).toContain('Never add secret values');
  });

  test('refuses every collision and leaves existing content unchanged', async () => {
    const parent = await mkdtemp(join(tmpdir(), 'woml-init-collision-'));
    temporary.push(parent);
    const target = join(parent, 'project');
    await initializeWomlProject(target);
    const before = await readFile(join(target, 'workflow.json'), 'utf8');

    await expect(initializeWomlProject(target)).rejects.toMatchObject({
      code: 'WOML_PROJECT_INIT_COLLISION',
    } satisfies Partial<WomlProjectInitializationError>);
    expect(await readFile(join(target, 'workflow.json'), 'utf8')).toBe(
      before
    );
  });

  test('packages the exact frozen public schema and template assets', async () => {
    const cliRoot = resolve(import.meta.dir, '..');
    for (const schema of [
      'woml-project.v1.schema.json',
      'project-check.v1.schema.json',
    ]) {
      expect(
        await readFile(resolve(cliRoot, 'schemas', schema), 'utf8')
      ).toBe(
        await readFile(resolve(cliRoot, '../docs/schemas', schema), 'utf8')
      );
    }
    expect(await readFile(resolve(cliRoot, 'templates/workflow.json'), 'utf8'))
      .toContain('"profile": "woml.project/v1"');
    expect(await readFile(resolve(cliRoot, 'templates/AGENTS.md'), 'utf8'))
      .toContain('Keep authority operator-owned');
  });
});

