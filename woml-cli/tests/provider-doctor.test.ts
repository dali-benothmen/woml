import { describe, expect, test } from 'bun:test';

import { runCli, type CliDependencies, type CliIo } from '../src/cli';
import {
  formatProviderDoctor,
  parseProviderDoctorArguments,
  runProviderDoctor,
} from '../src/provider-doctor';
import type { SecretMetadata, SecretStore } from '../src/secrets';

class MemorySecrets implements SecretStore {
  readonly provider = 'environment' as const;

  constructor(readonly values: Readonly<Record<string, string>>) {}

  async get(name: string): Promise<string | undefined> {
    return this.values[name];
  }
  async has(name: string): Promise<boolean> {
    return this.values[name] !== undefined;
  }
  async list(): Promise<readonly SecretMetadata[]> {
    return Object.keys(this.values).map(name => ({ name, provider: this.provider }));
  }
  async set(): Promise<void> {
    throw new Error('read only');
  }
  async delete(): Promise<boolean> {
    return false;
  }
}

function jsonResponse(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function capture(): { io: CliIo; stdout: string[]; stderr: string[] } {
  const stdout: string[] = [];
  const stderr: string[] = [];
  return {
    stdout,
    stderr,
    io: {
      stdout: value => stdout.push(value),
      stderr: value => stderr.push(value),
      isTTY: false,
    },
  };
}

describe('provider doctor diagnostics', () => {
  test('checks Telegram authentication, polling conflicts, and optional chat access', async () => {
    const token = 'telegram-super-secret';
    const requested: string[] = [];
    const result = await runProviderDoctor(
      parseProviderDoctorArguments([
        'telegram',
        'doctor',
        '--destination',
        '-1001234567890',
      ]),
      {
        secretStore: new MemorySecrets({ TELEGRAM_BOT_TOKEN: token }),
        fetch: (async input => {
          const url = String(input);
          requested.push(url);
          if (url.endsWith('/getMe')) {
            return jsonResponse({ ok: true, result: { id: 1, is_bot: true } });
          }
          if (url.endsWith('/getWebhookInfo')) {
            return jsonResponse({ ok: true, result: { url: '' } });
          }
          return jsonResponse({ ok: true, result: { id: -1001234567890 } });
        }) as typeof fetch,
      }
    );
    expect(result.status).toBe('healthy');
    expect(result.checks.map(item => item.id)).toEqual([
      'credentials',
      'authentication',
      'polling',
      'destination',
    ]);
    expect(requested).toHaveLength(3);
    expect(JSON.stringify(result)).not.toContain(token);
    expect(formatProviderDoctor(result)).not.toContain(token);
  });

  test('reports Telegram webhook conflicts without exposing provider payloads', async () => {
    const result = await runProviderDoctor(
      parseProviderDoctorArguments(['telegram', 'doctor']),
      {
        secretStore: new MemorySecrets({ TELEGRAM_BOT_TOKEN: 'secret' }),
        fetch: (async input =>
          String(input).endsWith('/getMe')
            ? jsonResponse({ ok: true, result: { id: 1, is_bot: true } })
            : jsonResponse({
                ok: true,
                result: { url: 'https://private.example/secret-callback' },
              })) as typeof fetch,
      }
    );
    expect(result.status).toBe('failed');
    expect(result.checks).toContainEqual(
      expect.objectContaining({ code: 'WOML_TELEGRAM_POLLING_CONFLICT' })
    );
    expect(JSON.stringify(result)).not.toContain('private.example');
  });

  test('checks Discord identity, intents, and channel visibility', async () => {
    const result = await runProviderDoctor(
      parseProviderDoctorArguments([
        'discord',
        'doctor',
        '--destination',
        '200000000000000001',
      ]),
      {
        secretStore: new MemorySecrets({ DISCORD_BOT_TOKEN: 'discord-secret' }),
        fetch: (async input => {
          const url = String(input);
          if (url.endsWith('/users/@me')) {
            return jsonResponse({ id: '100000000000000001', username: 'WOML', bot: true });
          }
          if (url.endsWith('/oauth2/applications/@me')) {
            return jsonResponse({ id: '100000000000000002', flags: 1 << 18 });
          }
          return jsonResponse({ id: '200000000000000001' });
        }) as typeof fetch,
      }
    );
    expect(result.status).toBe('healthy');
    expect(result.checks).toContainEqual(
      expect.objectContaining({ code: 'WOML_DISCORD_MESSAGE_CONTENT_READY' })
    );
  });

  test('checks WhatsApp credentials, phone identity, and public callback shape', async () => {
    const accessToken = 'whatsapp-secret-access-token';
    const result = await runProviderDoctor(
      parseProviderDoctorArguments([
        'whatsapp',
        'doctor',
        '--phone-number-id',
        '123456789012345',
        '--callback-url',
        'https://automation.example/callbacks/whatsapp',
      ]),
      {
        secretStore: new MemorySecrets({
          WHATSAPP_ACCESS_TOKEN: accessToken,
          WHATSAPP_APP_SECRET: 'private-application-value',
          WHATSAPP_VERIFY_TOKEN: 'private-verification-value',
        }),
        fetch: (async (_input, init) => {
          expect(init?.headers).toEqual({
            authorization: `Bearer ${accessToken}`,
          });
          return jsonResponse({ id: '123456789012345', verified_name: 'WOML' });
        }) as typeof fetch,
      }
    );
    expect(result.status).toBe('healthy');
    expect(JSON.stringify(result)).not.toMatch(
      /whatsapp-secret-access-token|private-application-value|private-verification-value/
    );
  });

  test('bounds malformed provider responses and preserves the CLI JSON contract', async () => {
    const store = new MemorySecrets({ TELEGRAM_BOT_TOKEN: 'secret-token' });
    const dependencies: CliDependencies = {
      createSecretStore: () => store,
      readSecret: async () => '',
      fetch: async () =>
        new Response('x'.repeat(65 * 1024), {
          headers: { 'content-type': 'application/json' },
        }),
    };
    const output = capture();
    const code = await runCli(
      ['telegram', 'doctor', '--json'],
      output.io,
      dependencies
    );
    expect(code).toBe(1);
    expect(output.stderr).toEqual([]);
    const result = JSON.parse(output.stdout.join(''));
    expect(result).toMatchObject({
      profile: 'woml.provider-doctor/v1',
      provider: 'telegram',
      status: 'failed',
    });
    expect(JSON.stringify(result)).not.toContain('secret-token');
  });

  test('rejects invalid provider-specific flags with usage and no network request', async () => {
    expect(() =>
      parseProviderDoctorArguments([
        'discord',
        'doctor',
        '--destination',
        'general',
      ])
    ).toThrow('WOML_CLI_ARGUMENTS_INVALID');
    expect(() =>
      parseProviderDoctorArguments([
        'telegram',
        'doctor',
        '--phone-number-id',
        '123',
      ])
    ).toThrow('WOML_CLI_ARGUMENTS_INVALID');
  });

  test('supports colored, plain, and machine-readable diagnostics', () => {
    const result = {
      profile: 'woml.provider-doctor/v1',
      provider: 'telegram',
      status: 'healthy',
      checks: [{
        id: 'authentication',
        status: 'pass',
        code: 'WOML_TELEGRAM_AUTH_READY',
        message: 'Telegram authenticated the configured bot.',
      }],
    } as const;
    expect(formatProviderDoctor(result, { color: 'always' })).toContain('\u001b[');
    expect(formatProviderDoctor(result, { color: 'never' })).not.toContain('\u001b[');
    expect(formatProviderDoctor(result, {
      color: 'auto',
      isTTY: true,
      environment: { TERM: 'xterm', NO_COLOR: '' },
    })).not.toContain('\u001b[');
    expect(JSON.stringify(result)).not.toContain('\u001b[');
  });
});
