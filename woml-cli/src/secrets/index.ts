import { BunSecretStore } from './bun-secret-store';
import { EnvironmentSecretStore } from './environment-secret-store';
import { MountedFileSecretStore } from './mounted-file-secret-store';
import { ProductionSecretStore } from './production-secret-store';
import {
  SecretStoreError,
  WOML_SECRET_DIRECTORY_ENV,
  WOML_SECRET_PROVIDER_ENV,
  type SecretStore,
} from './types';

export * from './bun-secret-store';
export * from './environment-secret-store';
export * from './mounted-file-secret-store';
export * from './production-secret-store';
export * from './types';

export function createSecretStore(
  environment: Readonly<Record<string, string | undefined>> = process.env
): SecretStore {
  const provider = environment[WOML_SECRET_PROVIDER_ENV] ?? 'os';
  if (provider === 'os') return new BunSecretStore();
  if (provider === 'env') return new EnvironmentSecretStore(environment);
  if (provider === 'files') {
    const directory = environment[WOML_SECRET_DIRECTORY_ENV];
    if (directory === undefined || directory.length === 0) {
      throw new SecretStoreError(
        'WOML_SECRET_PROVIDER_INVALID',
        'WOML_SECRETS_DIRECTORY is required when WOML_SECRETS_PROVIDER="files".'
      );
    }
    return new MountedFileSecretStore(directory);
  }
  if (provider === 'production') {
    const sources: SecretStore[] = [];
    const directory = environment[WOML_SECRET_DIRECTORY_ENV];
    if (directory !== undefined && directory.length > 0) {
      sources.push(new MountedFileSecretStore(directory));
    }
    sources.push(new EnvironmentSecretStore(environment), new BunSecretStore());
    return new ProductionSecretStore(sources);
  }
  throw new SecretStoreError(
    'WOML_SECRET_PROVIDER_INVALID',
    'WOML_SECRETS_PROVIDER must be "os", "env", "files", or "production".'
  );
}
