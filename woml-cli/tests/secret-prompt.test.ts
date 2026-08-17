import { describe, expect, test } from 'bun:test';
import { EventEmitter } from 'node:events';

import {
  readSecretFromTerminal,
  type SecretPromptInput,
} from '../src/secrets/prompt';
import { SecretStoreError } from '../src/secrets/types';

class FakeSecretInput extends EventEmitter implements SecretPromptInput {
  readonly isTTY = true;
  isRaw = false;
  readonly rawModeChanges: boolean[] = [];
  resumed = false;
  paused = false;

  setRawMode(enabled: boolean) {
    this.isRaw = enabled;
    this.rawModeChanges.push(enabled);
  }

  resume() {
    this.resumed = true;
  }

  pause() {
    this.paused = true;
  }
}

describe('secret terminal prompt', () => {
  test('shows stars, erases a star on backspace, and returns the hidden value', async () => {
    const input = new FakeSecretInput();
    let output = '';
    const result = readSecretFromTerminal('SLACK_BOT_TOKEN', {
      input,
      write: text => {
        output += text;
      },
    });

    input.emit('data', Buffer.from('ab'));
    input.emit('data', Buffer.from('\u007fc\n'));

    expect(await result).toBe('ac');
    expect(output).toBe('Enter value for SLACK_BOT_TOKEN: **\b \b*\n');
    expect(output).not.toContain('ab');
    expect(input.rawModeChanges).toEqual([true, false]);
    expect(input.resumed).toBe(true);
    expect(input.paused).toBe(true);
  });

  test('restores the terminal after cancellation without revealing input', async () => {
    const input = new FakeSecretInput();
    let output = '';
    const result = readSecretFromTerminal('TOKEN', {
      input,
      write: text => {
        output += text;
      },
    });

    input.emit('data', Buffer.from('secret\u0003'));

    try {
      await result;
      throw new Error('Expected secret entry to be cancelled.');
    } catch (error) {
      expect(error).toBeInstanceOf(SecretStoreError);
      expect((error as SecretStoreError).code).toBe(
        'WOML_SECRET_PROMPT_CANCELLED'
      );
    }
    expect(output).toBe('Enter value for TOKEN: ******\n');
    expect(output).not.toContain('secret');
    expect(input.rawModeChanges).toEqual([true, false]);
    expect(input.paused).toBe(true);
  });
});
