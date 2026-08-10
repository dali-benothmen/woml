import { describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';

const workflowPath = resolve(
  import.meta.dir,
  '../../woml/tests/fixtures/modules/customer-import.woml'
);

async function invoke(args: readonly string[]) {
  let stdout = '';
  let stderr = '';
  const io: CliIo = {
    stdout: text => {
      stdout += text;
    },
    stderr: text => {
      stderr += text;
    },
  };
  const exitCode = await runCli(args, io);
  return { exitCode, stdout, stderr };
}

describe('MS2 module compilation CLI', () => {
  test('checks, compiles, and explains a deterministic local module graph', async () => {
    const result = await invoke(['check', workflowPath]);
    expect(result.exitCode).toBe(0);
    expect(result.stderr).toBe('');
    expect(result.stdout).toContain('WOML check passed');
    expect(result.stdout).toContain('services.spreadsheet');
    expect(result.stdout).toContain('(read, removeEmptyRows)');
    expect(result.stdout).toContain('executable module package ready');
  });

  test('prints the reviewed executable package as JSON without activating code', async () => {
    const result = await invoke(['check', workflowPath, '--json']);
    expect(result.exitCode).toBe(0);
    expect(result.stderr).toBe('');
    const manifest = JSON.parse(result.stdout);
    expect(manifest).toMatchObject({
      schemaVersion: 2,
      profile: 'woml.definition-package/v2',
      executable: true,
      runtimeReady: false,
      workflow: { id: 'customer-import' },
      modules: [
        {
          name: 'spreadsheet',
          exports: ['read', 'removeEmptyRows'],
        },
      ],
    });
  });

  test('keeps woml run fail-closed until module execution reaches MS3', async () => {
    const result = await invoke(['run', workflowPath]);
    expect(result.exitCode).toBe(1);
    expect(result.stdout).toBe('');
    expect(result.stderr).toContain('WOML_MODULE_EXECUTION_UNAVAILABLE');
  });
});
