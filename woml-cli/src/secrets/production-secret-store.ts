import {
  requireValidSecretName,
  SecretStoreError,
  type SecretMetadata,
  type SecretStore,
} from './types';

/**
 * Resolves only requested symbolic names. Source order is significant, but a
 * different value in two sources is rejected instead of being hidden by it.
 */
export class ProductionSecretStore implements SecretStore {
  readonly provider = 'production' as const;
  readonly #sources: readonly SecretStore[];

  constructor(sources: readonly SecretStore[]) {
    if (sources.length === 0) {
      throw new SecretStoreError(
        'WOML_SECRET_PROVIDER_INVALID',
        'The production secret provider requires at least one source.'
      );
    }
    this.#sources = sources;
  }

  async get(name: string): Promise<string | undefined> {
    requireValidSecretName(name);
    const found: { provider: string; value: string }[] = [];
    for (const source of this.#sources) {
      const value = await source.get(name);
      if (value !== undefined) found.push({ provider: source.provider, value });
    }
    if (found.length === 0) return undefined;
    if (found.some(item => item.value !== found[0]!.value)) {
      throw new SecretStoreError(
        'WOML_SECRET_SOURCE_CONFLICT',
        `Secret ${name} has conflicting values in configured production sources: ${found.map(item => item.provider).join(', ')}.`
      );
    }
    return found[0]!.value;
  }

  async has(name: string): Promise<boolean> {
    return (await this.get(name)) !== undefined;
  }

  async list(): Promise<readonly SecretMetadata[]> {
    const names = new Set<string>();
    for (const source of this.#sources) {
      for (const item of await source.list()) names.add(item.name);
    }
    const result: SecretMetadata[] = [];
    for (const name of [...names].sort()) {
      await this.get(name);
      result.push({ name, provider: this.provider });
    }
    return result;
  }

  async set(_name: string, _value: string): Promise<void> {
    throw this.#readOnlyError();
  }

  async delete(_name: string): Promise<boolean> {
    throw this.#readOnlyError();
  }

  #readOnlyError(): SecretStoreError {
    return new SecretStoreError(
      'WOML_SECRET_PROVIDER_READ_ONLY',
      'The production secret provider is read-only. Rotate the selected environment, mounted-file, or OS source directly.'
    );
  }
}
