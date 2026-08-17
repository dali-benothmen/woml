import { SecretStoreError } from './types';

export interface SecretPromptInput {
  readonly isTTY?: boolean;
  readonly isRaw?: boolean;
  readonly setRawMode?: (enabled: boolean) => unknown;
  readonly resume: () => unknown;
  readonly pause: () => unknown;
  readonly on: (
    event: 'data',
    listener: (chunk: Buffer | string) => void
  ) => unknown;
  readonly off: (
    event: 'data',
    listener: (chunk: Buffer | string) => void
  ) => unknown;
}

export interface SecretPromptTerminal {
  readonly input: SecretPromptInput;
  readonly write: (text: string) => unknown;
}

const processTerminal: SecretPromptTerminal = {
  input: process.stdin,
  write: text => process.stderr.write(text),
};

export async function readSecretFromTerminal(
  name: string,
  terminal: SecretPromptTerminal = processTerminal
): Promise<string> {
  const { input, write } = terminal;
  const setRawMode = input.setRawMode;
  if (!input.isTTY || typeof setRawMode !== 'function') {
    throw new SecretStoreError(
      'WOML_SECRET_PROMPT_REQUIRES_TTY',
      '`woml secrets set` requires an interactive terminal and never accepts a secret as a command argument.'
    );
  }

  write(`Enter value for ${name}: `);
  const wasRaw = input.isRaw ?? false;
  setRawMode.call(input, true);
  input.resume();

  return await new Promise<string>((resolve, reject) => {
    let value = '';
    const finish = (error?: Error) => {
      input.off('data', onData);
      setRawMode.call(input, wasRaw);
      input.pause();
      write('\n');
      if (error === undefined) resolve(value);
      else reject(error);
    };
    const onData = (chunk: Buffer | string) => {
      const text = chunk.toString('utf8');
      for (const character of text) {
        if (character === '\r' || character === '\n') {
          finish();
          return;
        }
        if (character === '\u0003') {
          finish(
            new SecretStoreError(
              'WOML_SECRET_PROMPT_CANCELLED',
              'Secret entry was cancelled.'
            )
          );
          return;
        }
        if (character === '\u007f' || character === '\b') {
          const characters = Array.from(value);
          if (characters.length > 0) {
            characters.pop();
            value = characters.join('');
            write('\b \b');
          }
        } else if (character >= ' ') {
          value += character;
          write('*');
        }
      }
    };
    input.on('data', onData);
  });
}
