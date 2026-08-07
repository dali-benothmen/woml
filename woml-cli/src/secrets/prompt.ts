import { SecretStoreError } from './types';

export async function readSecretFromTerminal(name: string): Promise<string> {
  const input = process.stdin;
  if (!input.isTTY || typeof input.setRawMode !== 'function') {
    throw new SecretStoreError(
      'WOML_SECRET_PROMPT_REQUIRES_TTY',
      '`woml secrets set` requires an interactive terminal and never accepts a secret as a command argument.'
    );
  }

  process.stderr.write(`Enter value for ${name}: `);
  const wasRaw = input.isRaw;
  input.setRawMode(true);
  input.resume();

  return await new Promise<string>((resolve, reject) => {
    let value = '';
    const finish = (error?: Error) => {
      input.off('data', onData);
      input.setRawMode(wasRaw);
      input.pause();
      process.stderr.write('\n');
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
          value = value.slice(0, -1);
        } else if (character >= ' ') {
          value += character;
        }
      }
    };
    input.on('data', onData);
  });
}
