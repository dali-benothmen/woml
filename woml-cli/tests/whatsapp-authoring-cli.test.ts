import { describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';

const fixture = resolve(
  import.meta.dir,
  '../../woml/tests/fixtures/communication-providers/whatsapp.woml'
);

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

describe('WhatsApp CLI boundary', () => {
  test('checks WhatsApp authoring without contacting Meta', async () => {
    const result = await invoke(['check', fixture]);
    expect(result.exitCode).toBe(0);
    expect(result.stderr).toBe('');
    expect(result.stdout).toContain('WOML check passed');
    expect(result.stdout).toContain('WhatsApp triggers, approved-template notifications');
    expect(result.stdout).toContain('WhatsApp triggers, approved-template notifications');
    expect(result.stdout).toContain('are executable through the durable Rust runtime');
    expect(result.stdout).toContain('/callbacks/whatsapp');
  });

});
