import { isValidSecretName, type SecretReferenceExpression } from 'woml';

export const WOML_SECRET_SERVICE = 'dev.woml.cli.secrets.v1';
export const WOML_SECRET_ENV_PREFIX = 'WOML_SECRET_';
export const WOML_SECRET_PROVIDER_ENV = 'WOML_SECRETS_PROVIDER';
export const WOML_SECRET_DIRECTORY_ENV = 'WOML_SECRETS_DIRECTORY';
export const WOML_SECRET_MAX_BYTES = 2048;

export type SecretProvider =
  | 'os-keychain'
  | 'environment'
  | 'mounted-files'
  | 'production';

export interface SecretMetadata {
  readonly name: string;
  readonly provider: SecretProvider;
  readonly updatedAt?: string;
}

export interface SecretStore {
  readonly provider: SecretProvider;
  get(name: string): Promise<string | undefined>;
  has(name: string): Promise<boolean>;
  list(): Promise<readonly SecretMetadata[]>;
  set(name: string, value: string): Promise<void>;
  delete(name: string): Promise<boolean>;
}

export class SecretStoreError extends Error {
  readonly code: string;

  constructor(code: string, message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = 'SecretStoreError';
    this.code = code;
  }
}

export function requireValidSecretName(name: string): void {
  if (isValidSecretName(name)) return;
  throw new SecretStoreError(
    'WOML_SECRET_NAME_INVALID',
    'Secret names must start with A-Z and contain only A-Z, 0-9, or underscore.'
  );
}

export function requireValidSecretValue(value: string): void {
  const bytes = new TextEncoder().encode(value).byteLength;
  if (bytes === 0) {
    throw new SecretStoreError(
      'WOML_SECRET_VALUE_EMPTY',
      'Secret values must not be empty.'
    );
  }
  if (bytes > WOML_SECRET_MAX_BYTES) {
    throw new SecretStoreError(
      'WOML_SECRET_VALUE_TOO_LARGE',
      `Secret values must not exceed ${WOML_SECRET_MAX_BYTES} UTF-8 bytes.`
    );
  }
}

export async function preflightSecretReferences(
  references: readonly SecretReferenceExpression[],
  store: SecretStore
): Promise<void> {
  const missing: string[] = [];
  for (const name of [
    ...new Set(references.map(reference => reference.name)),
  ]) {
    requireValidSecretName(name);
    if (!(await store.has(name))) missing.push(name);
  }
  if (missing.length > 0) {
    throw new SecretStoreError(
      'WOML_SECRET_NOT_FOUND',
      `Missing required secret${missing.length === 1 ? '' : 's'}: ${missing.join(', ')}.`
    );
  }
}
