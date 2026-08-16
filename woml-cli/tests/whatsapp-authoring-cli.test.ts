import { describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';

import { runCli, type CliIo } from '../src/cli';

const fixture = resolve(
  import.meta.dir,
  '../../woml/tests/fixtures/communication-providers/whatsapp-acp6.woml'
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

describe('ACP6 WhatsApp CLI boundary', () => {
  test('checks WhatsApp authoring without contacting Meta', async () => {
    const result = await invoke(['check', fixture]);
    expect(result.exitCode).toBe(0);
    expect(result.stderr).toBe('');
    expect(result.stdout).toContain('WOML check passed');
    expect(result.stdout).toContain('WhatsApp trigger, approved-template notifications');
    expect(result.stdout).toContain('Execution: WhatsApp transport activates in ACP7');
    expect(result.stdout).toContain('/callbacks/whatsapp');
  });

  test('fails run before resolving credentials or contacting Meta', async () => {
    const result = await invoke(['run', fixture]);
    expect(result.exitCode).toBe(1);
    expect(result.stdout).toBe('');
    expect(result.stderr).toContain('WOML_COMMUNICATION_RUNTIME_UNAVAILABLE');
    expect(result.stderr).toContain('arrives in ACP7');
  });
});
