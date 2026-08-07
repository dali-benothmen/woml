import {
  requireValidSecretName,
  requireValidSecretValue,
  SecretStoreError,
  WOML_SECRET_SERVICE,
  type SecretMetadata,
  type SecretStore,
} from './types';

const indexName = '__woml_metadata_index_v1__';

interface StoredIndex {
  readonly version: 1;
  readonly secrets: readonly {
    readonly name: string;
    readonly updatedAt: string;
  }[];
}

interface BunSecretsApi {
  get(options: { service: string; name: string }): Promise<string | null>;
  set(options: {
    service: string;
    name: string;
    value: string;
    allowUnrestrictedAccess?: boolean;
  }): Promise<void>;
  delete(options: { service: string; name: string }): Promise<boolean>;
}

function defaultApi(): BunSecretsApi {
  const api = (Bun as unknown as { secrets?: BunSecretsApi }).secrets;
  if (api !== undefined) return api;
  throw new SecretStoreError(
    'WOML_SECRET_STORE_UNAVAILABLE',
    'This Bun runtime does not provide OS-native secret storage. Upgrade to a Bun release with Bun.secrets support.'
  );
}

function safeStoreError(error: unknown): SecretStoreError {
  if (error instanceof SecretStoreError) return error;
  return new SecretStoreError(
    'WOML_SECRET_STORE_UNAVAILABLE',
    'The operating-system credential store is unavailable or locked.',
    { cause: error }
  );
}

export class BunSecretStore implements SecretStore {
  readonly provider = 'os-keychain' as const;
  readonly #api: BunSecretsApi;
  readonly #now: () => Date;

  constructor(
    api: BunSecretsApi = defaultApi(),
    now: () => Date = () => new Date()
  ) {
    this.#api = api;
    this.#now = now;
  }

  async get(name: string): Promise<string | undefined> {
    requireValidSecretName(name);
    try {
      return (
        (await this.#api.get({ service: WOML_SECRET_SERVICE, name })) ??
        undefined
      );
    } catch (error) {
      throw safeStoreError(error);
    }
  }

  async has(name: string): Promise<boolean> {
    return (await this.get(name)) !== undefined;
  }

  async list(): Promise<readonly SecretMetadata[]> {
    const index = await this.#readIndex();
    return index.secrets.map(secret => ({
      name: secret.name,
      provider: this.provider,
      updatedAt: secret.updatedAt,
    }));
  }

  async set(name: string, value: string): Promise<void> {
    requireValidSecretName(name);
    requireValidSecretValue(value);
    try {
      const index = await this.#readIndex();
      await this.#api.set({
        service: WOML_SECRET_SERVICE,
        name,
        value,
        allowUnrestrictedAccess: false,
      });
      const next = index.secrets.filter(secret => secret.name !== name);
      next.push({ name, updatedAt: this.#now().toISOString() });
      next.sort((left, right) => left.name.localeCompare(right.name));
      await this.#writeIndex({ version: 1, secrets: next });
    } catch (error) {
      throw safeStoreError(error);
    }
  }

  async delete(name: string): Promise<boolean> {
    requireValidSecretName(name);
    try {
      const index = await this.#readIndex();
      const deleted = await this.#api.delete({
        service: WOML_SECRET_SERVICE,
        name,
      });
      if (index.secrets.some(secret => secret.name === name)) {
        await this.#writeIndex({
          version: 1,
          secrets: index.secrets.filter(secret => secret.name !== name),
        });
      }
      return deleted;
    } catch (error) {
      throw safeStoreError(error);
    }
  }

  async #readIndex(): Promise<StoredIndex> {
    let encoded: string | null;
    try {
      encoded = await this.#api.get({
        service: WOML_SECRET_SERVICE,
        name: indexName,
      });
    } catch (error) {
      throw safeStoreError(error);
    }
    if (encoded === null) return { version: 1, secrets: [] };

    try {
      const parsed = JSON.parse(encoded) as Partial<StoredIndex>;
      if (parsed.version !== 1 || !Array.isArray(parsed.secrets)) throw null;
      const seen = new Set<string>();
      const secrets = parsed.secrets.map(entry => {
        if (
          typeof entry !== 'object' ||
          entry === null ||
          !('name' in entry) ||
          !('updatedAt' in entry) ||
          typeof entry.name !== 'string' ||
          typeof entry.updatedAt !== 'string' ||
          !Number.isFinite(Date.parse(entry.updatedAt)) ||
          seen.has(entry.name)
        ) {
          throw null;
        }
        requireValidSecretName(entry.name);
        seen.add(entry.name);
        return { name: entry.name, updatedAt: entry.updatedAt };
      });
      return { version: 1, secrets };
    } catch (error) {
      throw new SecretStoreError(
        'WOML_SECRET_INDEX_INVALID',
        'The protected WOML secret metadata index is invalid.',
        { cause: error }
      );
    }
  }

  async #writeIndex(index: StoredIndex): Promise<void> {
    await this.#api.set({
      service: WOML_SECRET_SERVICE,
      name: indexName,
      value: JSON.stringify(index),
      allowUnrestrictedAccess: false,
    });
  }
}
