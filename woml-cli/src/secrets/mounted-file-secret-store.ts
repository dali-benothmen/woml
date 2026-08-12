import { lstat, readdir, readFile, realpath } from 'node:fs/promises';
import { isAbsolute, join, resolve } from 'node:path';

import {
  requireValidSecretName,
  requireValidSecretValue,
  SecretStoreError,
  type SecretMetadata,
  type SecretStore,
} from './types';

function unsafe(message: string, cause?: unknown): SecretStoreError {
  return new SecretStoreError('WOML_SECRET_FILE_UNSAFE', message, { cause });
}

export class MountedFileSecretStore implements SecretStore {
  readonly provider = 'mounted-files' as const;
  readonly #directory: string;

  constructor(directory: string) {
    if (!isAbsolute(directory)) {
      throw unsafe('WOML_SECRETS_DIRECTORY must be an absolute path.');
    }
    this.#directory = resolve(directory);
  }

  async get(name: string): Promise<string | undefined> {
    requireValidSecretName(name);
    await this.#validateDirectory();
    const path = join(this.#directory, name);
    let entry;
    try {
      entry = await lstat(path);
    } catch (error) {
      if (error instanceof Error && 'code' in error && error.code === 'ENOENT')
        return undefined;
      throw unsafe(`Mounted secret ${name} could not be inspected.`, error);
    }
    if (entry.isSymbolicLink() || !entry.isFile()) {
      throw unsafe(`Mounted secret ${name} must be a regular file, not a link.`);
    }
    if (process.platform !== 'win32') {
      const currentUid = typeof process.getuid === 'function' ? process.getuid() : undefined;
      if (currentUid !== undefined && entry.uid !== currentUid && entry.uid !== 0) {
        throw unsafe(`Mounted secret ${name} must be owned by the runtime user or root.`);
      }
      if ((entry.mode & 0o077) !== 0) {
        throw unsafe(`Mounted secret ${name} must not be accessible by group or other users.`);
      }
    }
    if (entry.size > 2049) {
      throw new SecretStoreError(
        'WOML_SECRET_VALUE_TOO_LARGE',
        'Mounted secret values must not exceed 2048 UTF-8 bytes.'
      );
    }
    let value = await readFile(path, 'utf8');
    if (value.endsWith('\r\n')) value = value.slice(0, -2);
    else if (value.endsWith('\n')) value = value.slice(0, -1);
    if (value.includes('\0')) {
      throw unsafe(`Mounted secret ${name} contains a NUL byte.`);
    }
    requireValidSecretValue(value);
    return value;
  }

  async has(name: string): Promise<boolean> {
    return (await this.get(name)) !== undefined;
  }

  async list(): Promise<readonly SecretMetadata[]> {
    await this.#validateDirectory();
    const names = (await readdir(this.#directory)).sort();
    const result: SecretMetadata[] = [];
    for (const name of names) {
      try {
        requireValidSecretName(name);
      } catch {
        continue;
      }
      if ((await this.get(name)) !== undefined) {
        result.push({ name, provider: this.provider });
      }
    }
    return result;
  }

  async set(_name: string, _value: string): Promise<void> {
    throw this.#readOnlyError();
  }

  async delete(_name: string): Promise<boolean> {
    throw this.#readOnlyError();
  }

  async #validateDirectory(): Promise<void> {
    let entry;
    try {
      entry = await lstat(this.#directory);
      if (entry.isSymbolicLink() || !entry.isDirectory()) throw null;
      if ((await realpath(this.#directory)) !== this.#directory) throw null;
    } catch (error) {
      throw unsafe('WOML_SECRETS_DIRECTORY must be a real, existing directory.', error);
    }
    if (process.platform !== 'win32') {
      const currentUid = typeof process.getuid === 'function' ? process.getuid() : undefined;
      if (currentUid !== undefined && entry.uid !== currentUid && entry.uid !== 0) {
        throw unsafe('WOML_SECRETS_DIRECTORY must be owned by the runtime user or root.');
      }
      if ((entry.mode & 0o022) !== 0) {
        throw unsafe('WOML_SECRETS_DIRECTORY must not be writable by group or other users.');
      }
    }
  }

  #readOnlyError(): SecretStoreError {
    return new SecretStoreError(
      'WOML_SECRET_PROVIDER_READ_ONLY',
      'The mounted-file secret provider is read-only. Update the mounted secret through the deployment platform.'
    );
  }
}
