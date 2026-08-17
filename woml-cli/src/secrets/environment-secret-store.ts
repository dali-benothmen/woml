import {
  requireValidSecretName,
  SecretStoreError,
  WOML_SECRET_ENV_PREFIX,
  type SecretMetadata,
  type SecretStore,
} from './types';

export class EnvironmentSecretStore implements SecretStore {
  readonly provider = 'environment' as const;
  readonly #environment: Readonly<Record<string, string | undefined>>;

  constructor(
    environment: Readonly<Record<string, string | undefined>> = process.env
  ) {
    this.#environment = environment;
  }

  async get(name: string): Promise<string | undefined> {
    requireValidSecretName(name);
    const value = this.#environment[`${WOML_SECRET_ENV_PREFIX}${name}`];
    return value === '' ? undefined : value;
  }

  async has(name: string): Promise<boolean> {
    return (await this.get(name)) !== undefined;
  }

  async list(): Promise<readonly SecretMetadata[]> {
    return Object.keys(this.#environment)
      .filter(key => key.startsWith(WOML_SECRET_ENV_PREFIX))
      .map(key => key.slice(WOML_SECRET_ENV_PREFIX.length))
      .filter(requireValidEnvironmentName)
      .filter(
        name => this.#environment[`${WOML_SECRET_ENV_PREFIX}${name}`] !== ''
      )
      .sort()
      .map(name => ({ name, provider: this.provider }));
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
      'The environment secret provider is read-only. Configure WOML_SECRET_<NAME> in the CI secret manager.'
    );
  }
}

function requireValidEnvironmentName(name: string): boolean {
  try {
    requireValidSecretName(name);
    return true;
  } catch {
    return false;
  }
}
