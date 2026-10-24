import { afterEach, describe, expect, test } from 'bun:test';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { runCli, type CliIo } from '../src/cli';
import { initializeWomlProject } from '../src/project-init';

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
    io: { stdout: value => stdout.push(value), stderr: value => stderr.push(value) },
    stdout,
    stderr,
  };
}

async function initializedProject() {
  const root = await mkdtemp(join(tmpdir(), 'woml-project-preflight-'));
  temporary.push(root);
  return await initializeWomlProject(join(root, 'preflight'));
}

describe('unified WOML project preflight', () => {
  test('emits equivalent ready JSON and human summaries without activation', async () => {
    const initialized = await initializedProject();
    const json = capture();
    expect(
      await runCli(
        ['check', '--project', initialized.manifestPath, '--json'],
        json.io
      )
    ).toBe(0);
    const report = JSON.parse(json.stdout.join(''));
    expect(report.status).toBe('ready');
    expect(report.workflows).toEqual([
      expect.objectContaining({ id: 'hello', status: 'ready' }),
    ]);
    expect(report.subsystems).toContainEqual(
      expect.objectContaining({ id: 'runtime', status: 'ready' })
    );

    const human = capture();
    expect(
      await runCli(['check', '--project', initialized.manifestPath], human.io)
    ).toBe(0);
    expect(human.stdout.join('')).toContain('Ready to run');
    expect(human.stdout.join('')).toContain('1 compiled');
  });

  test('blocks a process workflow without operator-owned process authority', async () => {
    const initialized = await initializedProject();
    await writeFile(
      initialized.workflowPath,
      `<woml>\n  <workflow id="process-check">\n    <triggers><manual id="start" /></triggers>\n    <steps>\n      <step id="run"><script>return await services.process.run({ executable: "bun", args: ["--version"] });</script></step>\n    </steps>\n  </workflow>\n</woml>\n`
    );
    const result = capture();
    expect(
      await runCli(
        ['check', '--project', initialized.manifestPath, '--json'],
        result.io
      )
    ).toBe(1);
    const report = JSON.parse(result.stdout.join(''));
    expect(report.status).toBe('blocked');
    expect(report.subsystems).toContainEqual(
      expect.objectContaining({
        id: 'process',
        status: 'blocked',
        required: true,
      })
    );
    expect(report.diagnostics).toContainEqual(
      expect.objectContaining({ code: 'WOML_PROCESS_DISABLED' })
    );
  });

  test('checks every workflow and preserves source locations for invalid ones', async () => {
    const initialized = await initializedProject();
    const invalidPath = join(initialized.directory, 'workflows', 'invalid.woml');
    await writeFile(
      invalidPath,
      `<woml>\n  <workflow id="invalid">\n    <triggers><manual id="start" /></triggers>\n    <bogus />\n  </workflow>\n</woml>\n`
    );
    const result = capture();
    expect(
      await runCli(
        ['check', '--project', initialized.manifestPath, '--json'],
        result.io
      )
    ).toBe(1);
    const report = JSON.parse(result.stdout.join(''));
    expect(report.workflows).toContainEqual(
      expect.objectContaining({ id: 'hello', status: 'ready' })
    );
    expect(report.diagnostics).toContainEqual(
      expect.objectContaining({
        code: 'WOML_UNKNOWN_ELEMENT',
        sourcePath: invalidPath,
        line: 4,
        column: 5,
      })
    );
  });
});

