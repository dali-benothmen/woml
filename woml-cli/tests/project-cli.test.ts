import { afterEach, describe, expect, test } from 'bun:test';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { runCli, type CliIo } from '../src/cli';
import { initializeWomlProject } from '../src/project-init';

const temporary: string[] = [];
const originalDirectory = process.cwd();

afterEach(async () => {
  process.chdir(originalDirectory);
  await Promise.all(
    temporary.splice(0).map(path => rm(path, { recursive: true, force: true }))
  );
});

function output(): { io: CliIo; stdout: string[]; stderr: string[] } {
  const stdout: string[] = [];
  const stderr: string[] = [];
  return {
    io: { stdout: value => stdout.push(value), stderr: value => stderr.push(value) },
    stdout,
    stderr,
  };
}

async function project(): Promise<Awaited<ReturnType<typeof initializeWomlProject>>> {
  const root = await mkdtemp(join(tmpdir(), 'woml-project-cli-'));
  temporary.push(root);
  return await initializeWomlProject(join(root, 'demo'));
}

describe('project-aware core commands', () => {
  test('initializes through the public CLI and checks an explicit project', async () => {
    const root = await mkdtemp(join(tmpdir(), 'woml-project-cli-init-'));
    temporary.push(root);
    const target = join(root, 'demo');
    const initializedOutput = output();
    expect(await runCli(['init', target, '--json'], initializedOutput.io)).toBe(0);
    expect(JSON.parse(initializedOutput.stdout.join('')).profile).toBe(
      'woml.project-initialization/v1'
    );

    const checked = output();
    expect(
      await runCli(
        ['check', '--project', join(target, 'workflow.json'), '--json'],
        checked.io
      )
    ).toBe(0);
    expect(JSON.parse(checked.stdout.join(''))).toMatchObject({
      profile: 'woml.project-check/v1',
      status: 'ready',
      project: { name: 'demo' },
    });
  });

  test('discovers only the exact current-directory manifest', async () => {
    const initialized = await project();
    process.chdir(initialized.directory);
    const discovered = output();
    expect(await runCli(['check', '--json'], discovered.io)).toBe(0);
    expect(JSON.parse(discovered.stdout.join('')).project.manifestPath).toBe(
      initialized.manifestPath
    );

    process.chdir(join(initialized.directory, 'workflows'));
    const nested = output();
    expect(await runCli(['check'], nested.io)).toBe(2);
    expect(nested.stderr.join('')).toContain('Usage: woml check');
  });

  test('rejects positional/project and authority collisions deterministically', async () => {
    const initialized = await project();
    const positional = output();
    expect(
      await runCli(
        [
          'check',
          initialized.workflowPath,
          '--project',
          initialized.manifestPath,
        ],
        positional.io
      )
    ).toBe(2);
    expect(positional.stderr.join('')).toContain('workflow inputs or --project');

    const manifest = JSON.parse(
      await readFile(initialized.manifestPath, 'utf8')
    ) as Record<string, unknown>;
    manifest.runtime = './runtime.json';
    await writeFile(
      initialized.manifestPath,
      `${JSON.stringify(manifest, null, 2)}\n`
    );
    await writeFile(
      join(initialized.directory, 'runtime.json'),
      `${JSON.stringify({ profile: 'woml.runtime/v1', version: 1 })}\n`
    );
    const conflict = output();
    expect(
      await runCli(
        [
          'check',
          '--project',
          initialized.manifestPath,
          '--config',
          join(initialized.directory, 'runtime.json'),
        ],
        conflict.io
      )
    ).toBe(1);
    expect(conflict.stderr.join('')).toContain('WOML_PROJECT_OPTION_CONFLICT');
  });

  test('generates editor types from the project workflow set', async () => {
    const initialized = await project();
    const target = join(initialized.directory, 'generated.d.ts');
    const result = output();
    expect(
      await runCli(
        ['types', '--project', initialized.manifestPath, '--output', target],
        result.io
      )
    ).toBe(0);
    expect(await readFile(target, 'utf8')).toContain(
      'interface WomlBuiltinServices'
    );
  });

  test('uses the discovered project inputs for run and reports the public selection', async () => {
    const initialized = await project();
    await writeFile(
      initialized.workflowPath,
      `<woml>\n  <workflow id="process-check">\n    <triggers><manual id="start" /></triggers>\n    <steps><step id="run"><script>return await services.process.run({ executable: "bun", args: ["--version"] });</script></step></steps>\n  </workflow>\n</woml>\n`
    );
    process.chdir(initialized.directory);
    const result = output();
    expect(await runCli(['run', '--json'], result.io)).toBe(1);
    const selection = JSON.parse(result.stdout[0]!);
    expect(selection).toMatchObject({
      profile: 'woml.project-selection/v1',
      project: 'demo',
      selectedBy: 'current-directory',
    });
    expect(selection.digest).toMatch(/^sha256:[a-f0-9]{64}$/u);
    expect(result.stderr.join('')).toContain('WOML_PROCESS_DISABLED');
  });
});

