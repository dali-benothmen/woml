import { describe, expect, test } from 'bun:test';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { runCli, type CliIo } from '../src/cli';

async function check(source: string, ...options: string[]) {
  const directory = await mkdtemp(join(tmpdir(), 'woml-for-each-check-'));
  const file = join(directory, 'workflow.woml');
  const stdout: string[] = [];
  const stderr: string[] = [];
  const io: CliIo = {
    stdout: text => stdout.push(text),
    stderr: text => stderr.push(text),
  };
  try {
    await writeFile(file, source, 'utf8');
    const exitCode = await runCli(['check', file, ...options], io);
    return { exitCode, stdout: stdout.join(''), stderr: stderr.join('') };
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}

const valid = `<woml>
  <workflow id="for-each-check" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps>
      <step id="load"><script>return { items: [1, 2] };</script></step>
      <for-each id="processItems" items="{{context.steps.load.items}}" concurrency="2">
        <step id="processItem"><script>return context.item;</script></step>
      </for-each>
    </steps>
  </workflow>
</woml>`;

describe('woml check for-each authoring', () => {
  test('accepts valid source and reports bounded concurrent Rust execution', async () => {
    const result = await check(valid);
    expect(result.exitCode).toBe(0);
    expect(result.stderr).toBe('');
    expect(result.stdout).toContain('WOML check passed');
    expect(result.stdout).toContain('Compiled Model v16');
    expect(result.stdout).toContain('bounded concurrent Rust execution');
  });

  test('returns the Model v16 validation profile for JSON output', async () => {
    const result = await check(valid, '--json');
    expect(result.exitCode).toBe(0);
    expect(JSON.parse(result.stdout)).toEqual({
      profile: 'woml.model-validation/v1',
      valid: true,
      workflowId: 'for-each-check',
      feature: 'for-each',
      executable: true,
      modelVersion: 16,
    });
  });

  test('keeps precise validation diagnostics for invalid loops', async () => {
    const result = await check(
      valid.replace(' items="{{context.steps.load.items}}"', '')
    );
    expect(result.exitCode).toBe(1);
    expect(result.stdout).toBe('');
    expect(result.stderr).toContain('WOML_FOR_EACH_ITEMS_REQUIRED');
    expect(result.stderr).toContain('workflow.woml:');
  });
});
