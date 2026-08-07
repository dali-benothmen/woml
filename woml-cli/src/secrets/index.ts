import { BunSecretStore } from './bun-secret-store';
import { EnvironmentSecretStore } from './environment-secret-store';
import {
  SecretStoreError,
  WOML_SECRET_PROVIDER_ENV,
  type SecretStore,
} from './types';

export * from './bun-secret-store';
export * from './environment-secret-store';
export * from './types';

export function createSecretStore(
  environment: Readonly<Record<string, string | undefined>> = process.env
): SecretStore {
  const provider = environment[WOML_SECRET_PROVIDER_ENV] ?? 'os';
  if (provider === 'os') return new BunSecretStore();
  if (provider === 'env') return new EnvironmentSecretStore(environment);
  throw new SecretStoreError(
    'WOML_SECRET_PROVIDER_INVALID',
    'WOML_SECRETS_PROVIDER must be either "os" or "env".'
  );
}
