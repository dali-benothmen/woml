import { createHash, randomUUID } from 'node:crypto';
import { constants, createReadStream } from 'node:fs';
import {
  chmod,
  copyFile,
  lstat,
  mkdir,
  open,
  readdir,
  readFile,
  rename,
  rm,
  unlink,
  writeFile,
} from 'node:fs/promises';
import { basename, dirname, join, parse, resolve } from 'node:path';

import {
  BackupOperationError,
  createBackupWithRust,
  inspectBackupStoreWithRust,
  prepareRestoredStoreWithRust,
  recordVerifiedBackupWithRust,
  type RustBackupStoreInspection,
} from './rust-executor';
import {
  readRuntimeDescriptor,
  runtimeDescriptorPath,
} from './runtime-control';

export const backupUsage =
  'Usage: woml backup <backup-directory> [--state <path>] [--json]';
export const restoreUsage =
  'Usage: woml restore <backup-directory> [--state <path>] [--replace] [--json]';

export interface BackupManifestV1 {
  readonly profile: 'woml.backup-manifest/v1';
  readonly backupId: string;
  readonly createdAt: string;
  readonly deploymentId: string;
  readonly activationId: string;
  readonly storeVersion: number;
  readonly database: {
    readonly file: 'state.sqlite';
    readonly sizeBytes: number;
    readonly digest: string;
  };
  readonly definitionHashes: readonly string[];
  readonly verified: true;
}

export interface BackupArguments {
  readonly backupDirectory: string;
  readonly statePath: string;
  readonly json: boolean;
}

export interface RestoreArguments extends BackupArguments {
  readonly replace: boolean;
}

export class ProductionBackupError extends Error {
  readonly code: string;

  constructor(code: string, message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = 'ProductionBackupError';
    this.code = code;
  }
}

function parseArguments(
  args: readonly string[],
  command: 'backup' | 'restore'
): BackupArguments | RestoreArguments {
  const usage = command === 'backup' ? backupUsage : restoreUsage;
  const [, rawDirectory, ...options] = args;
  if (
    args[0] !== command ||
    rawDirectory === undefined ||
    rawDirectory.length === 0 ||
    rawDirectory.startsWith('--')
  ) {
    throw new ProductionBackupError('WOML_CLI_ARGUMENTS_INVALID', usage);
  }
  let statePath = resolve('.woml/state.sqlite');
  let json = false;
  let replace = false;
  const seen = new Set<string>();
  for (let index = 0; index < options.length; index += 1) {
    const option = options[index]!;
    if (seen.has(option)) {
      throw new ProductionBackupError('WOML_CLI_ARGUMENTS_INVALID', usage);
    }
    seen.add(option);
    if (option === '--json') {
      json = true;
      continue;
    }
    if (option === '--replace' && command === 'restore') {
      replace = true;
      continue;
    }
    if (option !== '--state') {
      throw new ProductionBackupError('WOML_CLI_ARGUMENTS_INVALID', usage);
    }
    const value = options[++index];
    if (value === undefined || value.length === 0 || value.startsWith('--')) {
      throw new ProductionBackupError('WOML_CLI_ARGUMENTS_INVALID', usage);
    }
    statePath = resolve(value);
  }
  const base = {
    backupDirectory: resolve(rawDirectory),
    statePath,
    json,
  };
  return command === 'restore' ? { ...base, replace } : base;
}

export function parseBackupArguments(args: readonly string[]): BackupArguments {
  return parseArguments(args, 'backup') as BackupArguments;
}

export function parseRestoreArguments(args: readonly string[]): RestoreArguments {
  return parseArguments(args, 'restore') as RestoreArguments;
}

async function exists(path: string): Promise<boolean> {
  try {
    await lstat(path);
    return true;
  } catch (error) {
    if (error instanceof Error && 'code' in error && error.code === 'ENOENT') return false;
    throw error;
  }
}

async function assertNoSymlink(path: string): Promise<void> {
  const absolute = resolve(path);
  const root = parse(absolute).root;
  const pending: string[] = [];
  let cursor = absolute;
  while (cursor !== root) {
    pending.push(cursor);
    cursor = dirname(cursor);
  }
  for (const candidate of pending.reverse()) {
    try {
      if ((await lstat(candidate)).isSymbolicLink()) {
        throw new ProductionBackupError(
          'WOML_BACKUP_PATH_UNSAFE',
          `Backup and restore paths cannot contain symbolic links: ${candidate}`
        );
      }
    } catch (error) {
      if (error instanceof ProductionBackupError) throw error;
      if (!(error instanceof Error && 'code' in error && error.code === 'ENOENT')) throw error;
    }
  }
}

async function digestFile(path: string): Promise<{ digest: string; sizeBytes: number }> {
  const hasher = createHash('sha256');
  let sizeBytes = 0;
  for await (const chunk of createReadStream(path)) {
    const bytes = chunk as Buffer;
    sizeBytes += bytes.byteLength;
    hasher.update(bytes);
  }
  if (sizeBytes < 1) {
    throw new ProductionBackupError(
      'WOML_BACKUP_VERIFICATION_FAILED',
      'The backup database is empty.'
    );
  }
  return { digest: `sha256:${hasher.digest('hex')}`, sizeBytes };
}

async function syncFile(path: string): Promise<void> {
  const handle = await open(path, 'r');
  try {
    await handle.sync();
  } finally {
    await handle.close();
  }
}

async function syncDirectory(path: string): Promise<void> {
  let handle;
  try {
    handle = await open(path, 'r');
    await handle.sync();
  } catch (error) {
    if (
      process.platform === 'win32' &&
      error instanceof Error &&
      'code' in error &&
      ['EISDIR', 'EINVAL', 'ENOTSUP', 'EPERM'].includes(String(error.code))
    ) {
      return;
    }
    throw error;
  } finally {
    await handle?.close();
  }
}

function deploymentIdentity(statePath: string): string {
  return `deployment_${createHash('sha256')
    .update(resolve(statePath))
    .digest('hex')
    .slice(0, 24)}`;
}

function isDigest(value: unknown): value is string {
  return typeof value === 'string' && /^sha256:[0-9a-f]{64}$/.test(value);
}

function decodeManifest(value: unknown): BackupManifestV1 {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new ProductionBackupError('WOML_BACKUP_MANIFEST_INVALID', 'Backup manifest must be an object.');
  }
  const item = value as Record<string, unknown>;
  const database = item.database;
  const keys = Object.keys(item).sort().join(',');
  if (
    keys !== ['activationId', 'backupId', 'createdAt', 'database', 'definitionHashes', 'deploymentId', 'profile', 'storeVersion', 'verified'].sort().join(',') ||
    item.profile !== 'woml.backup-manifest/v1' ||
    typeof item.backupId !== 'string' ||
    item.backupId.length < 1 ||
    item.backupId.length > 320 ||
    typeof item.createdAt !== 'string' ||
    !Number.isFinite(Date.parse(item.createdAt)) ||
    typeof item.deploymentId !== 'string' ||
    item.deploymentId.length < 1 ||
    item.deploymentId.length > 320 ||
    !isDigest(item.activationId) ||
    ![13, 14].includes(Number(item.storeVersion)) ||
    database === null ||
    typeof database !== 'object' ||
    Array.isArray(database) ||
    Object.keys(database).sort().join(',') !== ['digest', 'file', 'sizeBytes'].sort().join(',') ||
    (database as Record<string, unknown>).file !== 'state.sqlite' ||
    !Number.isSafeInteger((database as Record<string, unknown>).sizeBytes) ||
    Number((database as Record<string, unknown>).sizeBytes) < 1 ||
    !isDigest((database as Record<string, unknown>).digest) ||
    !Array.isArray(item.definitionHashes) ||
    item.definitionHashes.length < 1 ||
    item.definitionHashes.length > 10_000 ||
    !item.definitionHashes.every(isDigest) ||
    new Set(item.definitionHashes).size !== item.definitionHashes.length ||
    item.verified !== true
  ) {
    throw new ProductionBackupError(
      'WOML_BACKUP_MANIFEST_INVALID',
      'Backup Manifest v1 is malformed or unsupported.'
    );
  }
  return value as BackupManifestV1;
}

function sameInventory(
  manifest: BackupManifestV1,
  inspection: RustBackupStoreInspection
): boolean {
  return manifest.storeVersion === inspection.storeVersion &&
    JSON.stringify(manifest.definitionHashes) === JSON.stringify(inspection.definitionHashes);
}

export async function verifyBackupDirectory(
  backupDirectory: string,
  nativeCorePath?: string
): Promise<BackupManifestV1> {
  await assertNoSymlink(backupDirectory);
  const entry = await lstat(backupDirectory).catch(() => undefined);
  if (entry === undefined || !entry.isDirectory()) {
    throw new ProductionBackupError('WOML_BACKUP_NOT_FOUND', 'The backup directory does not exist.');
  }
  const entries = (await readdir(backupDirectory)).sort();
  if (entries.length !== 2 || entries[0] !== 'manifest.json' || entries[1] !== 'state.sqlite') {
    throw new ProductionBackupError(
      'WOML_BACKUP_INCOMPLETE',
      'The backup directory must contain exactly manifest.json and state.sqlite.'
    );
  }
  const manifestPath = join(backupDirectory, 'manifest.json');
  const databasePath = join(backupDirectory, 'state.sqlite');
  await assertNoSymlink(manifestPath);
  await assertNoSymlink(databasePath);
  const manifestEntry = await lstat(manifestPath).catch(() => undefined);
  const databaseEntry = await lstat(databasePath).catch(() => undefined);
  if (!manifestEntry?.isFile() || !databaseEntry?.isFile()) {
    throw new ProductionBackupError(
      'WOML_BACKUP_INCOMPLETE',
      'The backup must contain regular manifest.json and state.sqlite files.'
    );
  }
  let decoded: unknown;
  try {
    decoded = JSON.parse(await readFile(manifestPath, 'utf8'));
  } catch (error) {
    throw new ProductionBackupError(
      'WOML_BACKUP_MANIFEST_INVALID',
      'Backup manifest is not valid JSON.',
      { cause: error }
    );
  }
  const manifest = decodeManifest(decoded);
  const measured = await digestFile(databasePath);
  if (
    measured.digest !== manifest.database.digest ||
    measured.sizeBytes !== manifest.database.sizeBytes
  ) {
    throw new ProductionBackupError(
      'WOML_BACKUP_CHECKSUM_MISMATCH',
      'The backup database checksum or byte size does not match its manifest.'
    );
  }
  let inspection: RustBackupStoreInspection;
  try {
    inspection = inspectBackupStoreWithRust(databasePath, { nativeCorePath });
  } catch (error) {
    if (error instanceof BackupOperationError) {
      throw new ProductionBackupError(error.code, error.message, { cause: error });
    }
    throw error;
  }
  if (!sameInventory(manifest, inspection)) {
    throw new ProductionBackupError(
      'WOML_BACKUP_INVENTORY_MISMATCH',
      'The backup definition inventory does not match its manifest.'
    );
  }
  return manifest;
}

export async function createProductionBackup(
  args: BackupArguments,
  options: { readonly nativeCorePath?: string } = {}
): Promise<BackupManifestV1> {
  await assertNoSymlink(args.statePath);
  const source = await lstat(args.statePath).catch(() => undefined);
  if (source === undefined || !source.isFile()) {
    throw new ProductionBackupError(
      'WOML_BACKUP_SOURCE_INVALID',
      `No durable WOML state database exists at ${args.statePath}.`
    );
  }
  await assertNoSymlink(args.backupDirectory);
  if (await exists(args.backupDirectory)) {
    throw new ProductionBackupError(
      'WOML_BACKUP_DESTINATION_EXISTS',
      'The backup destination already exists; choose a new directory.'
    );
  }
  const parent = dirname(args.backupDirectory);
  await mkdir(parent, { recursive: true, mode: 0o700 });
  await assertNoSymlink(parent);
  const temporaryDirectory = join(
    parent,
    `.${basename(args.backupDirectory)}.tmp-${randomUUID()}`
  );
  const backupId = `backup_${randomUUID().replaceAll('-', '')}`;
  const createdAt = new Date().toISOString();
  await mkdir(temporaryDirectory, { mode: 0o700 });
  const databasePath = join(temporaryDirectory, 'state.sqlite');
  try {
    let inventory: RustBackupStoreInspection;
    try {
      inventory = createBackupWithRust(
        args.statePath,
        databasePath,
        `lease_${randomUUID().replaceAll('-', '')}`,
        `backup_cli_${process.pid}_${randomUUID().replaceAll('-', '')}`,
        deploymentIdentity(args.statePath),
        { nativeCorePath: options.nativeCorePath }
      );
    } catch (error) {
      if (error instanceof BackupOperationError) {
        throw new ProductionBackupError(error.code, error.message, { cause: error });
      }
      throw error;
    }
    await chmod(databasePath, 0o600);
    const database = await digestFile(databasePath);
    if (inventory.deploymentId === undefined || inventory.activationId === undefined) {
      throw new ProductionBackupError(
        'WOML_BACKUP_VERIFICATION_FAILED',
        'The native backup did not provide deployment identity.'
      );
    }
    const manifest: BackupManifestV1 = {
      profile: 'woml.backup-manifest/v1',
      backupId,
      createdAt,
      deploymentId: inventory.deploymentId,
      activationId: inventory.activationId,
      storeVersion: inventory.storeVersion,
      database: {
        file: 'state.sqlite',
        sizeBytes: database.sizeBytes,
        digest: database.digest,
      },
      definitionHashes: inventory.definitionHashes,
      verified: true,
    };
    await writeFile(
      join(temporaryDirectory, 'manifest.json'),
      `${JSON.stringify(manifest, null, 2)}\n`,
      { encoding: 'utf8', mode: 0o600, flag: 'wx' }
    );
    await verifyBackupDirectory(temporaryDirectory, options.nativeCorePath);
    await syncFile(databasePath);
    await syncFile(join(temporaryDirectory, 'manifest.json'));
    await syncDirectory(temporaryDirectory);
    await rename(temporaryDirectory, args.backupDirectory);
    await syncDirectory(parent);
    recordVerifiedBackupWithRust(args.statePath, backupId, createdAt, {
      nativeCorePath: options.nativeCorePath,
    });
    return manifest;
  } catch (error) {
    await rm(temporaryDirectory, { recursive: true, force: true }).catch(() => {});
    throw error;
  }
}

function processExists(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return !(error instanceof Error && 'code' in error && error.code === 'ESRCH');
  }
}

async function rejectActiveTarget(statePath: string, nativeCorePath?: string): Promise<void> {
  const descriptorPath = runtimeDescriptorPath(statePath);
  if (await exists(descriptorPath)) {
    const descriptor = await readRuntimeDescriptor(descriptorPath).catch(error => {
      throw new ProductionBackupError(
        'WOML_RESTORE_TARGET_UNSAFE',
        'The target runtime descriptor is invalid or unsafe.',
        { cause: error }
      );
    });
    if (processExists(descriptor.pid)) {
      throw new ProductionBackupError(
        'WOML_RESTORE_TARGET_ACTIVE',
        `Runtime ${descriptor.runtimeInstanceId} is still active; stop it before restore.`
      );
    }
  }
  if (!(await exists(statePath))) return;
  try {
    const inspection = inspectBackupStoreWithRust(statePath, { nativeCorePath });
    if (
      inspection.runtimeLeaseExpiresAt !== undefined &&
      Date.parse(inspection.runtimeLeaseExpiresAt) > Date.now()
    ) {
      throw new ProductionBackupError(
        'WOML_RESTORE_TARGET_ACTIVE',
        'The target store still has a live runtime ownership lease.'
      );
    }
  } catch (error) {
    if (error instanceof ProductionBackupError) throw error;
    if (error instanceof BackupOperationError) {
      throw new ProductionBackupError(
        'WOML_RESTORE_TARGET_UNSAFE',
        'The existing target is not a safely inspectable WOML store.',
        { cause: error }
      );
    }
    throw error;
  }
}

export async function restoreProductionBackup(
  args: RestoreArguments,
  options: { readonly nativeCorePath?: string } = {}
): Promise<{
  readonly profile: 'woml.restore-result/v1';
  readonly backupId: string;
  readonly statePath: string;
  readonly storeVersion: number;
  readonly rollbackPath?: string;
}> {
  const manifest = await verifyBackupDirectory(
    args.backupDirectory,
    options.nativeCorePath
  );
  await assertNoSymlink(args.statePath);
  await rejectActiveTarget(args.statePath, options.nativeCorePath);
  const targetExists = await exists(args.statePath);
  if (targetExists && !args.replace) {
    throw new ProductionBackupError(
      'WOML_RESTORE_CONFIRMATION_REQUIRED',
      'The target state database exists. Re-run with --replace after confirming the target and backup.'
    );
  }
  const targetDirectory = dirname(args.statePath);
  await mkdir(targetDirectory, { recursive: true, mode: 0o700 });
  await assertNoSymlink(targetDirectory);
  const temporaryPath = join(
    targetDirectory,
    `.${basename(args.statePath)}.restore-${randomUUID()}.tmp`
  );
  let rollbackPath: string | undefined;
  try {
    await copyFile(
      join(args.backupDirectory, manifest.database.file),
      temporaryPath,
      constants.COPYFILE_EXCL
    );
    await chmod(temporaryPath, 0o600);
    let prepared: RustBackupStoreInspection;
    try {
      prepared = prepareRestoredStoreWithRust(
        temporaryPath,
        manifest.definitionHashes,
        manifest.backupId,
        new Date().toISOString(),
        { nativeCorePath: options.nativeCorePath }
      );
    } catch (error) {
      if (error instanceof BackupOperationError) {
        throw new ProductionBackupError(error.code, error.message, { cause: error });
      }
      throw error;
    }
    if (
      prepared.storeVersion !== 14 ||
      JSON.stringify(prepared.definitionHashes) !== JSON.stringify(manifest.definitionHashes)
    ) {
      throw new ProductionBackupError(
        'WOML_RESTORE_VERIFICATION_FAILED',
        'The prepared restore target does not match the backup inventory.'
      );
    }
    await syncFile(temporaryPath);
    if (targetExists) {
      rollbackPath = `${args.statePath}.pre-restore-${Date.now()}`;
      await rename(args.statePath, rollbackPath);
      for (const suffix of ['-wal', '-shm']) {
        if (await exists(`${args.statePath}${suffix}`)) {
          await rename(`${args.statePath}${suffix}`, `${rollbackPath}${suffix}`);
        }
      }
    }
    try {
      await rename(temporaryPath, args.statePath);
    } catch (error) {
      if (rollbackPath !== undefined && !(await exists(args.statePath))) {
        await rename(rollbackPath, args.statePath).catch(() => {});
        for (const suffix of ['-wal', '-shm']) {
          if (await exists(`${rollbackPath}${suffix}`)) {
            await rename(`${rollbackPath}${suffix}`, `${args.statePath}${suffix}`).catch(
              () => {}
            );
          }
        }
      }
      throw error;
    }
    await chmod(args.statePath, 0o600);
    await syncFile(args.statePath);
    await syncDirectory(targetDirectory);
    return {
      profile: 'woml.restore-result/v1',
      backupId: manifest.backupId,
      statePath: args.statePath,
      storeVersion: prepared.storeVersion,
      ...(rollbackPath === undefined ? {} : { rollbackPath }),
    };
  } finally {
    await unlink(temporaryPath).catch(() => {});
  }
}
