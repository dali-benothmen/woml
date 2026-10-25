import { afterEach, describe, expect, test } from 'bun:test';
import { chmod, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import {
  compileWoml,
  parseWoml,
  WomlCompileError,
  WomlValidationError,
} from '@woml/compiler';

import { runCli, type CliIo } from '../src/cli';
import { initializeWomlProject } from '../src/project-init';

const repositoryRoot = resolve(import.meta.dir, '../..');
const temporary: string[] = [];

afterEach(async () => {
  await Promise.all(
    temporary.splice(0).map(path => rm(path, { recursive: true, force: true }))
  );
});

function capture(): { io: CliIo; stdout: string[]; stderr: string[] } {
  const stdout: string[] = [];
  const stderr: string[] = [];
  return {
    io: {
      stdout: value => stdout.push(value),
      stderr: value => stderr.push(value),
    },
    stdout,
    stderr,
  };
}

describe('combined project editor and AI-assisted authoring support', () => {
  test('packages the exact public project schema and associates the canonical filename', async () => {
    const publicSchema = await readFile(
      resolve(repositoryRoot, 'docs/schemas/woml-project.v1.schema.json'),
      'utf8'
    );
    for (const path of [
      'woml-cli/schemas/woml-project.v1.schema.json',
      'woml-vscode/schemas/woml-project.v1.schema.json',
    ]) {
      expect(await readFile(resolve(repositoryRoot, path), 'utf8')).toBe(
        publicSchema
      );
    }
    const extension = JSON.parse(
      await readFile(resolve(repositoryRoot, 'woml-vscode/package.json'), 'utf8')
    );
    expect(extension.contributes.jsonValidation).toContainEqual({
      fileMatch: '**/workflow.json',
      url: './schemas/woml-project.v1.schema.json',
    });
  });

  test('publishes combined feature metadata and safe snippets without a second parser', async () => {
    const metadata = JSON.parse(
      await readFile(
        resolve(repositoryRoot, 'docs/editor/woml-html-data.json'),
        'utf8'
      )
    ) as { tags: readonly { name: string; attributes?: readonly { name: string }[] }[] };
    const tags = new Map(metadata.tags.map(tag => [tag.name, tag]));
    for (const tag of [
      'approval',
      'lock',
      'saga',
      'compensate',
      'cleanup',
      'wait',
      'wait-for',
    ]) {
      expect(tags.has(tag), `missing HTML metadata for ${tag}`).toBe(true);
    }
    expect(tags.get('step')?.attributes?.map(item => item.name)).toContain(
      'target'
    );

    const snippets = JSON.parse(
      await readFile(
        resolve(repositoryRoot, 'woml-vscode/snippets/woml.code-snippets'),
        'utf8'
      )
    ) as Record<string, { prefix: string }>;
    for (const prefix of [
      'woml-safe-deployment',
      'woml-approval',
      'woml-lock',
      'woml-saga',
      'woml-wait',
      'woml-artifact',
      'woml-progress',
      'woml-remote-step',
    ]) {
      expect(Object.values(snippets).some(snippet => snippet.prefix === prefix)).toBe(
        true
      );
    }
  });

  test('initializes safety guidance and generates the actual script-host bindings', async () => {
    const parent = await mkdtemp(join(tmpdir(), 'woml-editor-project-'));
    temporary.push(parent);
    const initialized = await initializeWomlProject(join(parent, 'project'));
    const guide = await readFile(initialized.authoringGuidePath, 'utf8');
    expect(guide).toContain('Do not weaken, replace, inline,');
    expect(guide).toContain('Never add secret values');

    const declarationsPath = join(initialized.directory, 'woml-env.d.ts');
    const result = capture();
    expect(
      await runCli(
        [
          'types',
          '--project',
          initialized.manifestPath,
          '--output',
          declarationsPath,
        ],
        result.io
      )
    ).toBe(0);
    const declarations = await readFile(declarationsPath, 'utf8');
    for (const marker of [
      'declare const services',
      'declare const context',
      'declare const attempt',
      'declare const secrets',
      'readonly process: WomlProcessService',
      'readonly progress: WomlProgressService',
      'readonly artifacts: WomlArtifactsService',
      'readonly locks: WomlLocksService',
      'readonly fencingToken: string',
    ]) {
      expect(declarations).toContain(marker);
    }
  });

  test('project diagnostics preserve the compiler source code and location', async () => {
    const parent = await mkdtemp(join(tmpdir(), 'woml-editor-diagnostic-'));
    temporary.push(parent);
    const initialized = await initializeWomlProject(join(parent, 'project'));
    const source = `<woml>\n  <workflow id="invalid">\n    <triggers><manual id="start" /></triggers>\n    <bogus />\n  </workflow>\n</woml>\n`;
    await writeFile(initialized.workflowPath, source);

    let compilerDiagnostic:
      | WomlValidationError['diagnostic']
      | WomlCompileError['diagnostic']
      | undefined;
    try {
      compileWoml(parseWoml(source, { file: initialized.workflowPath }));
    } catch (error) {
      if (
        error instanceof WomlValidationError ||
        error instanceof WomlCompileError
      ) {
        compilerDiagnostic = error.diagnostic;
      } else {
        throw error;
      }
    }
    expect(compilerDiagnostic).toBeDefined();

    const checked = capture();
    expect(
      await runCli(
        ['check', '--project', initialized.manifestPath, '--json'],
        checked.io
      )
    ).toBe(1);
    const report = JSON.parse(checked.stdout.join(''));
    expect(report.diagnostics[0]).toMatchObject({
      code: compilerDiagnostic!.code,
      sourcePath: initialized.workflowPath,
      line: compilerDiagnostic!.location.start.line,
      column: compilerDiagnostic!.location.start.column,
    });
    expect(report.diagnostics[0].correctiveAction.length).toBeGreaterThan(0);
  });

  test('manifest-only edits cannot grant or weaken operator process authority', async () => {
    const parent = await mkdtemp(join(tmpdir(), 'woml-ai-policy-boundary-'));
    temporary.push(parent);
    const initialized = await initializeWomlProject(join(parent, 'project'));
    await writeFile(
      initialized.workflowPath,
      `<woml><workflow id="guarded"><triggers><manual id="start" /></triggers><steps><step id="effect"><script>return await services.process.run({ executable: "unreviewed", args: [] });</script></step></steps></workflow></woml>\n`
    );

    const removedAuthority = capture();
    expect(
      await runCli(
        ['check', '--project', initialized.manifestPath, '--json'],
        removedAuthority.io
      )
    ).toBe(1);
    expect(JSON.parse(removedAuthority.stdout.join('')).diagnostics).toContainEqual(
      expect.objectContaining({ code: 'WOML_PROCESS_DISABLED' })
    );

    const manifest = JSON.parse(await readFile(initialized.manifestPath, 'utf8'));
    manifest.authority = { allowProcess: true };
    await writeFile(initialized.manifestPath, `${JSON.stringify(manifest)}\n`);
    if (process.platform !== 'win32') await chmod(initialized.manifestPath, 0o600);
    const inlineGrant = capture();
    expect(
      await runCli(
        ['check', '--project', initialized.manifestPath, '--json'],
        inlineGrant.io
      )
    ).toBe(1);
    expect(inlineGrant.stderr.join('')).toContain('WOML_PROJECT_INVALID');
  });
});
