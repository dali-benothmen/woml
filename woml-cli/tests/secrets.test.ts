import { describe, expect, test } from 'bun:test';

import { runCli, type CliDependencies, type CliIo } from '../src/cli';
import { BunSecretStore } from '../src/secrets/bun-secret-store';
import { EnvironmentSecretStore } from '../src/secrets/environment-secret-store';
import {
  preflightSecretReferences,
  SecretStoreError,
  WOML_SECRET_SERVICE,
  type SecretStore,
} from '../src/secrets';

class MemorySecretStore implements SecretStore {
  readonly provider = 'os-keychain' as const;
  readonly values = new Map<string, string>();
  readonly updatedAt = new Map<string, string>();

  async get(name: string) {
    return this.values.get(name);
  }

  async has(name: string) {
    return this.values.has(name);
  }

  async list() {
    return [...this.values.keys()].sort().map(name => ({
      name,
      provider: this.provider,
      updatedAt: this.updatedAt.get(name),
    }));
  }

  async set(name: string, value: string) {
    this.values.set(name, value);
    this.updatedAt.set(name, '2026-08-07T12:00:00.000Z');
  }

  async delete(name: string) {
    this.updatedAt.delete(name);
    return this.values.delete(name);
  }
}

function capturedIo() {
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
  return { io, output: () => ({ stdout, stderr }) };
}

function dependencies(
  store: SecretStore,
  value = 'xoxb-test-value-that-must-not-be-printed'
): CliDependencies {
  return {
    createSecretStore: () => store,
    readSecret: async () => value,
  };
}

describe('woml secrets CLI', () => {
  test('sets, lists, rotates, and deletes without printing values', async () => {
    const store = new MemorySecretStore();
    const plaintext = 'xoxb-test-value-that-must-not-be-printed';

    let capture = capturedIo();
    expect(
      await runCli(
        ['secrets', 'set', 'SLACK_BOT_TOKEN'],
        capture.io,
        dependencies(store, plaintext)
      )
    ).toBe(0);
    expect(store.values.get('SLACK_BOT_TOKEN')).toBe(plaintext);
    expect(JSON.stringify(capture.output())).not.toContain(plaintext);
    expect(capture.output().stdout).toBe(
      'Stored secret SLACK_BOT_TOKEN in os-keychain.\n'
    );

    capture = capturedIo();
    expect(
      await runCli(['secrets', 'list'], capture.io, dependencies(store))
    ).toBe(0);
    expect(capture.output().stdout).toBe(
      'SLACK_BOT_TOKEN\tos-keychain\t2026-08-07T12:00:00.000Z\n'
    );
    expect(capture.output().stdout).not.toContain(plaintext);

    capture = capturedIo();
    expect(
      await runCli(
        ['secrets', 'delete', 'SLACK_BOT_TOKEN'],
        capture.io,
        dependencies(store)
      )
    ).toBe(0);
    expect(capture.output().stdout).toBe('Deleted secret SLACK_BOT_TOKEN.\n');
    expect(store.values.size).toBe(0);
  });

  test('rejects secret values as command arguments and invalid names', async () => {
    const store = new MemorySecretStore();
    let capture = capturedIo();
    expect(
      await runCli(
        ['secrets', 'set', 'SLACK_TOKEN', 'plaintext'],
        capture.io,
        dependencies(store)
      )
    ).toBe(2);
    expect(capture.output().stderr).toContain('woml secrets set <NAME>');
    expect(store.values.size).toBe(0);

    capture = capturedIo();
    expect(
      await runCli(
        ['secrets', 'set', 'slack_token'],
        capture.io,
        dependencies(store)
      )
    ).toBe(1);
    expect(capture.output().stderr).toContain('WOML_SECRET_NAME_INVALID');
  });

  test('reports a missing delete without exposing internal state', async () => {
    const capture = capturedIo();
    expect(
      await runCli(
        ['secrets', 'delete', 'MISSING_TOKEN'],
        capture.io,
        dependencies(new MemorySecretStore())
      )
    ).toBe(1);
    expect(capture.output().stderr).toBe(
      'WOML secrets error [WOML_SECRET_NOT_FOUND]: Secret MISSING_TOKEN is not configured.\n'
    );
  });

  test('does not prompt when the selected CI provider is read-only', async () => {
    const capture = capturedIo();
    let prompted = false;
    const store = new EnvironmentSecretStore({});

    expect(
      await runCli(['secrets', 'set', 'SLACK_BOT_TOKEN'], capture.io, {
        createSecretStore: () => store,
        readSecret: async () => {
          prompted = true;
          return 'must-not-be-read';
        },
      })
    ).toBe(1);
    expect(prompted).toBe(false);
    expect(capture.output().stderr).toContain('WOML_SECRET_PROVIDER_READ_ONLY');
  });
});

describe('secret store providers', () => {
  test('stores values and its metadata index only through the OS API', async () => {
    const keychain = new Map<string, string>();
    const api = {
      async get({ service, name }: { service: string; name: string }) {
        expect(service).toBe(WOML_SECRET_SERVICE);
        return keychain.get(name) ?? null;
      },
      async set({
        service,
        name,
        value,
        allowUnrestrictedAccess,
      }: {
        service: string;
        name: string;
        value: string;
        allowUnrestrictedAccess?: boolean;
      }) {
        expect(service).toBe(WOML_SECRET_SERVICE);
        expect(allowUnrestrictedAccess).toBe(false);
        keychain.set(name, value);
      },
      async delete({ service, name }: { service: string; name: string }) {
        expect(service).toBe(WOML_SECRET_SERVICE);
        return keychain.delete(name);
      },
    };
    const store = new BunSecretStore(
      api,
      () => new Date('2026-08-07T12:00:00.000Z')
    );

    await store.set('SLACK_APP_TOKEN', 'xapp-secret');
    expect(await store.get('SLACK_APP_TOKEN')).toBe('xapp-secret');
    expect(await store.list()).toEqual([
      {
        name: 'SLACK_APP_TOKEN',
        provider: 'os-keychain',
        updatedAt: '2026-08-07T12:00:00.000Z',
      },
    ]);
    expect([...keychain.values()].join('\n')).toContain('xapp-secret');
    expect(await store.delete('SLACK_APP_TOKEN')).toBe(true);
    expect(await store.has('SLACK_APP_TOKEN')).toBe(false);
  });

  test('provides an explicit read-only CI environment binding', async () => {
    const store = new EnvironmentSecretStore({
      WOML_SECRET_SLACK_BOT_TOKEN: 'xoxb-ci',
      WOML_SECRET_SLACK_APP_TOKEN: 'xapp-ci',
      WOML_SECRET_invalid: 'ignored',
      OTHER: 'ignored',
    });

    expect(await store.get('SLACK_BOT_TOKEN')).toBe('xoxb-ci');
    expect(await store.list()).toEqual([
      { name: 'SLACK_APP_TOKEN', provider: 'environment' },
      { name: 'SLACK_BOT_TOKEN', provider: 'environment' },
    ]);
    await expect(
      store.set('SLACK_BOT_TOKEN', 'replacement')
    ).rejects.toMatchObject({
      code: 'WOML_SECRET_PROVIDER_READ_ONLY',
    });
  });

  test('preflights unique symbolic names without returning their values', async () => {
    const store = new MemorySecretStore();
    await store.set('ONE', 'first');

    await expect(
      preflightSecretReferences(
        [
          { kind: 'secretReference', name: 'ONE' },
          { kind: 'secretReference', name: 'ONE' },
        ],
        store
      )
    ).resolves.toBeUndefined();
    await expect(
      preflightSecretReferences(
        [{ kind: 'secretReference', name: 'TWO' }],
        store
      )
    ).rejects.toEqual(
      new SecretStoreError(
        'WOML_SECRET_NOT_FOUND',
        'Missing required secret: TWO.'
      )
    );
  });
});
