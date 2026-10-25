import { readFile, realpath, stat } from 'node:fs/promises';
import { dirname, isAbsolute, relative, resolve, sep } from 'node:path';

import type { ProcessRuntimeMode } from './process-policy';

export interface ArtifactPolicyV1 {
  readonly contract: 'woml.artifact-policy';
  readonly contractVersion: 1;
  readonly enabled: boolean;
  readonly modes: { readonly foreground: boolean; readonly background: boolean };
  readonly projectRoot: string;
  readonly readRoots: readonly string[];
  readonly materialization: {
    readonly directory: string;
    readonly cleanupOnStart: true;
    readonly maxBytesPerAttempt: number;
  };
  readonly limits: {
    readonly maxArtifactBytes: number;
    readonly maxRunBytes: number;
    readonly maxStoreBytes: number;
    readonly maxConcurrentTransfers: number;
    readonly transferTimeoutMs: number;
  };
  readonly retention: {
    readonly defaultMs: number;
    readonly minimumMs: number;
    readonly maximumMs: number;
  };
  readonly secretScanning: {
    readonly knownSecretAction: 'reject' | 'quarantine' | 'allow';
    readonly minimumSecretBytes: 8;
  };
  readonly operationalOutput?: {
    readonly enabled: true;
    readonly retentionMs: number;
  };
  readonly backend: ArtifactBackendV1;
}

export type ArtifactBackendV1 =
  | {
      readonly contract: 'woml.artifact-backend';
      readonly contractVersion: 1;
      readonly kind: 'local';
      readonly directory: string;
    }
  | {
      readonly contract: 'woml.artifact-backend';
      readonly contractVersion: 1;
      readonly kind: 's3';
      readonly bucket: string;
      readonly region: string;
      readonly prefix: string;
      readonly endpoint?: string;
      readonly forcePathStyle: boolean;
      readonly authentication:
        | { readonly mode: 'ambient' }
        | {
            readonly mode: 'secrets';
            readonly accessKeySecret: string;
            readonly secretKeySecret: string;
            readonly sessionTokenSecret?: string;
          };
      readonly serverSideEncryption:
        | { readonly mode: 'aes256' }
        | { readonly mode: 'aws-kms'; readonly keyId: string };
    };

export interface ArtifactRuntimeAuthorityV1 {
  readonly artifacts: {
    readonly mode: ProcessRuntimeMode;
    readonly policy: ArtifactPolicyV1;
  };
}

export class ArtifactPolicyError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(`${message} Corrective action: provide a valid, operator-owned Artifact Policy v1 document.`);
    this.name = 'ArtifactPolicyError';
    this.code = code;
  }
}

export async function loadArtifactPolicy(
  path: string,
  mode: ProcessRuntimeMode
): Promise<ArtifactRuntimeAuthorityV1> {
  const policyPath = resolve(path);
  let source: string;
  try {
    const metadata = await stat(policyPath);
    if (!metadata.isFile() || metadata.size > 1024 * 1024) invalid('The artifact policy must be a JSON file no larger than 1 MiB.');
    source = await readFile(policyPath, 'utf8');
  } catch (error) {
    if (error instanceof ArtifactPolicyError) throw error;
    throw new ArtifactPolicyError(
      'WOML_ARTIFACT_POLICY_NOT_FOUND',
      `The artifact policy could not be read: ${policyPath}`
    );
  }
  let decoded: unknown;
  try {
    decoded = JSON.parse(source);
  } catch {
    invalid('The artifact policy is not valid JSON.');
  }
  return {
    artifacts: {
      mode,
      policy: await preflightArtifactPolicy(decoded, dirname(policyPath), mode),
    },
  };
}

export function artifactGrantSummary(authority: ArtifactRuntimeAuthorityV1): string {
  return `Artifact capability (${authority.artifacts.mode}): ${authority.artifacts.policy.backend.kind} immutable backend enabled`;
}

export function artifactPolicySecretNames(
  authority: Partial<ArtifactRuntimeAuthorityV1> | undefined
): readonly string[] {
  const backend = authority?.artifacts?.policy.backend;
  if (backend?.kind !== 's3' || backend.authentication.mode !== 'secrets') return [];
  return [
    backend.authentication.accessKeySecret,
    backend.authentication.secretKeySecret,
    ...(backend.authentication.sessionTokenSecret === undefined
      ? []
      : [backend.authentication.sessionTokenSecret]),
  ];
}

export async function preflightArtifactPolicy(
  input: unknown,
  baseDirectory: string,
  mode: ProcessRuntimeMode
): Promise<ArtifactPolicyV1> {
  const policy = strictObjectWithOptional(input, 'policy', [
    'contract', 'contractVersion', 'enabled', 'modes', 'projectRoot', 'readRoots',
    'materialization', 'limits', 'retention', 'secretScanning', 'backend',
  ], ['operationalOutput']);
  if (policy.contract !== 'woml.artifact-policy' || policy.contractVersion !== 1) invalid('The document does not match Artifact Policy v1.');
  if (policy.enabled !== true) throw new ArtifactPolicyError('WOML_ARTIFACT_SERVICE_DISABLED', 'Durable artifacts are disabled by policy.');
  const modes = strictObject(policy.modes, 'modes', ['foreground', 'background']);
  if (typeof modes.foreground !== 'boolean' || typeof modes.background !== 'boolean' || modes[mode] !== true) {
    throw new ArtifactPolicyError('WOML_ARTIFACT_SERVICE_DISABLED', `Durable artifacts are not enabled in ${mode} mode.`);
  }
  const projectRootValue = boundedString(policy.projectRoot, 'projectRoot', 4096);
  const projectRoot = await canonicalDirectory(resolve(baseDirectory, projectRootValue), 'projectRoot');
  if (!Array.isArray(policy.readRoots) || policy.readRoots.length < 1 || policy.readRoots.length > 128) invalid('readRoots must contain 1 through 128 paths.');
  const readRoots: string[] = [];
  for (const value of policy.readRoots) {
    const authored = boundedString(value, 'readRoots entry', 4096);
    const root = await canonicalDirectory(isAbsolute(authored) ? authored : resolve(projectRoot, authored), 'read root');
    const fromProject = relative(projectRoot, root);
    if (
      fromProject === '..' ||
      fromProject.startsWith(`..${sep}`) ||
      isAbsolute(fromProject)
    ) invalid('Every read root must remain beneath projectRoot.');
    if (!readRoots.includes(root)) readRoots.push(root);
  }
  const materialization = strictObject(policy.materialization, 'materialization', ['directory', 'cleanupOnStart', 'maxBytesPerAttempt']);
  if (materialization.cleanupOnStart !== true) invalid('materialization.cleanupOnStart must be true in v1.');
  const limits = strictObject(policy.limits, 'limits', ['maxArtifactBytes', 'maxRunBytes', 'maxStoreBytes', 'maxConcurrentTransfers', 'transferTimeoutMs']);
  const retention = strictObject(policy.retention, 'retention', ['defaultMs', 'minimumMs', 'maximumMs']);
  const secretScanning = strictObject(policy.secretScanning, 'secretScanning', ['knownSecretAction', 'minimumSecretBytes']);
  const operationalOutput = policy.operationalOutput === undefined
    ? undefined
    : strictObject(policy.operationalOutput, 'operationalOutput', ['enabled', 'retentionMs']);
  const backendValue = strictRecord(policy.backend, 'backend');
  const backend = normalizeBackend(backendValue, projectRoot);
  const normalized: ArtifactPolicyV1 = {
    contract: 'woml.artifact-policy', contractVersion: 1, enabled: true,
    modes: { foreground: modes.foreground as boolean, background: modes.background as boolean },
    projectRoot,
    readRoots,
    materialization: {
      directory: resolvePath(projectRoot, boundedString(materialization.directory, 'materialization.directory', 4096)),
      cleanupOnStart: true,
      maxBytesPerAttempt: integer(materialization.maxBytesPerAttempt, 1, 4_398_046_511_104, 'maxBytesPerAttempt'),
    },
    limits: {
      maxArtifactBytes: integer(limits.maxArtifactBytes, 1, 1_099_511_627_776, 'maxArtifactBytes'),
      maxRunBytes: integer(limits.maxRunBytes, 1, 4_398_046_511_104, 'maxRunBytes'),
      maxStoreBytes: integer(limits.maxStoreBytes, 1, 1_125_899_906_842_624, 'maxStoreBytes'),
      maxConcurrentTransfers: integer(limits.maxConcurrentTransfers, 1, 64, 'maxConcurrentTransfers'),
      transferTimeoutMs: integer(limits.transferTimeoutMs, 1000, 86_400_000, 'transferTimeoutMs'),
    },
    retention: {
      defaultMs: integer(retention.defaultMs, 3_600_000, 31_536_000_000, 'retention.defaultMs'),
      minimumMs: integer(retention.minimumMs, 3_600_000, 31_536_000_000, 'retention.minimumMs'),
      maximumMs: integer(retention.maximumMs, 3_600_000, 31_536_000_000, 'retention.maximumMs'),
    },
    secretScanning: {
      knownSecretAction: enumValue(secretScanning.knownSecretAction, ['reject', 'quarantine', 'allow'], 'knownSecretAction'),
      minimumSecretBytes: exactInteger(secretScanning.minimumSecretBytes, 8, 'minimumSecretBytes'),
    },
    ...(operationalOutput === undefined
      ? {}
      : {
          operationalOutput: {
            enabled: operationalOutput.enabled === true
              ? true as const
              : invalid('operationalOutput.enabled must be true when configured.'),
            retentionMs: integer(
              operationalOutput.retentionMs,
              3_600_000,
              31_536_000_000,
              'operationalOutput.retentionMs'
            ),
          },
        }),
    backend,
  };
  if (
    normalized.limits.maxRunBytes < normalized.limits.maxArtifactBytes ||
    normalized.limits.maxStoreBytes < normalized.limits.maxArtifactBytes ||
    normalized.retention.minimumMs > normalized.retention.defaultMs ||
    normalized.retention.defaultMs > normalized.retention.maximumMs ||
    (normalized.operationalOutput !== undefined &&
      (normalized.operationalOutput.retentionMs < normalized.retention.minimumMs ||
        normalized.operationalOutput.retentionMs > normalized.retention.maximumMs))
  ) invalid('Artifact limits and retention bounds are inconsistent.');
  return normalized;
}

function normalizeBackend(
  value: Record<string, unknown>,
  projectRoot: string
): ArtifactBackendV1 {
  if (value.kind === 'local') {
    const backend = strictObject(value, 'backend', [
      'contract', 'contractVersion', 'kind', 'directory',
    ]);
    if (backend.contract !== 'woml.artifact-backend' || backend.contractVersion !== 1) {
      invalid('The local backend does not match Artifact Backend v1.');
    }
    return {
      contract: 'woml.artifact-backend',
      contractVersion: 1,
      kind: 'local',
      directory: resolvePath(
        projectRoot,
        boundedString(backend.directory, 'backend.directory', 4096)
      ),
    };
  }
  const fields = [
    'contract', 'contractVersion', 'kind', 'bucket', 'region', 'prefix',
    'forcePathStyle', 'authentication', 'serverSideEncryption',
    ...(value.endpoint === undefined ? [] : ['endpoint']),
  ];
  const backend = strictObject(value, 'backend', fields);
  if (backend.contract !== 'woml.artifact-backend' || backend.contractVersion !== 1 || backend.kind !== 's3') {
    invalid('The backend does not match Artifact Backend v1.');
  }
  const bucket = boundedString(backend.bucket, 'backend.bucket', 63);
  if (!/^[a-z0-9][a-z0-9.-]{1,61}[a-z0-9]$/.test(bucket)) invalid('backend.bucket is invalid.');
  const region = boundedString(backend.region, 'backend.region', 128);
  const prefix = typeof backend.prefix === 'string' ? backend.prefix : invalid('backend.prefix is invalid.');
  if (Buffer.byteLength(prefix) > 512 || !/^(?:[A-Za-z0-9!_.*'()/-]+)?$/.test(prefix)) invalid('backend.prefix is invalid.');
  if (typeof backend.forcePathStyle !== 'boolean') invalid('backend.forcePathStyle must be boolean.');
  const endpoint = backend.endpoint === undefined
    ? undefined
    : boundedString(backend.endpoint, 'backend.endpoint', 2048);
  if (endpoint !== undefined) {
    let parsed: URL;
    try { parsed = new URL(endpoint); } catch { invalid('backend.endpoint must be a valid HTTPS URL.'); }
    if (
      parsed!.protocol !== 'https:' ||
      parsed!.username !== '' ||
      parsed!.password !== '' ||
      parsed!.search !== '' ||
      parsed!.hash !== ''
    ) {
      invalid('backend.endpoint must be a credential-free HTTPS URL.');
    }
  }
  const authenticationValue = strictRecord(backend.authentication, 'backend.authentication');
  let authentication: Extract<ArtifactBackendV1, { kind: 's3' }>['authentication'];
  if (authenticationValue.mode === 'ambient') {
    strictObject(authenticationValue, 'backend.authentication', ['mode']);
    authentication = { mode: 'ambient' };
  } else {
    const authenticationFields = [
      'mode', 'accessKeySecret', 'secretKeySecret',
      ...(authenticationValue.sessionTokenSecret === undefined ? [] : ['sessionTokenSecret']),
    ];
    const authored = strictObject(authenticationValue, 'backend.authentication', authenticationFields);
    if (authored.mode !== 'secrets') invalid('backend.authentication.mode is invalid.');
    const accessKeySecret = secretName(authored.accessKeySecret, 'accessKeySecret');
    const secretKeySecret = secretName(authored.secretKeySecret, 'secretKeySecret');
    authentication = {
      mode: 'secrets', accessKeySecret, secretKeySecret,
      ...(authored.sessionTokenSecret === undefined
        ? {}
        : { sessionTokenSecret: secretName(authored.sessionTokenSecret, 'sessionTokenSecret') }),
    };
  }
  const encryptionValue = strictRecord(backend.serverSideEncryption, 'backend.serverSideEncryption');
  const serverSideEncryption: Extract<ArtifactBackendV1, { kind: 's3' }>['serverSideEncryption'] =
    encryptionValue.mode === 'aes256'
      ? (strictObject(encryptionValue, 'backend.serverSideEncryption', ['mode']), { mode: 'aes256' })
      : (() => {
          const authored = strictObject(encryptionValue, 'backend.serverSideEncryption', ['mode', 'keyId']);
          if (authored.mode !== 'aws-kms') invalid('backend.serverSideEncryption.mode is invalid.');
          return { mode: 'aws-kms' as const, keyId: boundedString(authored.keyId, 'backend.serverSideEncryption.keyId', 2048) };
        })();
  return {
    contract: 'woml.artifact-backend', contractVersion: 1, kind: 's3',
    bucket, region, prefix, ...(endpoint === undefined ? {} : { endpoint }),
    forcePathStyle: backend.forcePathStyle, authentication, serverSideEncryption,
  };
}

function secretName(value: unknown, label: string): string {
  const name = boundedString(value, `backend.authentication.${label}`, 128);
  if (!/^[A-Z][A-Z0-9_]{0,127}$/.test(name)) invalid(`backend.authentication.${label} is invalid.`);
  return name;
}

function strictRecord(value: unknown, label: string): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) invalid(`${label} must be an object.`);
  return value as Record<string, unknown>;
}

function strictObject(value: unknown, label: string, fields: readonly string[]): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) invalid(`${label} must be an object.`);
  const object = value as Record<string, unknown>;
  if (Object.keys(object).some(key => !fields.includes(key)) || fields.some(key => !(key in object))) invalid(`${label} has missing or unknown fields.`);
  return object;
}
function strictObjectWithOptional(
  value: unknown,
  label: string,
  required: readonly string[],
  optional: readonly string[]
): Record<string, unknown> {
  const object = strictRecord(value, label);
  const allowed = new Set([...required, ...optional]);
  if (
    Object.keys(object).some(key => !allowed.has(key)) ||
    required.some(key => !(key in object))
  ) invalid(`${label} has missing or unknown fields.`);
  return object;
}
function boundedString(value: unknown, label: string, max: number): string {
  if (typeof value !== 'string' || value.length < 1 || Buffer.byteLength(value) > max) invalid(`${label} is invalid.`);
  return value;
}
function integer(value: unknown, min: number, max: number, label: string): number {
  if (!Number.isSafeInteger(value) || (value as number) < min || (value as number) > max) invalid(`${label} is outside the v1 bounds.`);
  return value as number;
}
function exactInteger<T extends number>(value: unknown, expected: T, label: string): T {
  if (value !== expected) invalid(`${label} must be ${expected}.`);
  return expected;
}
function enumValue<T extends string>(value: unknown, choices: readonly T[], label: string): T {
  if (typeof value !== 'string' || !choices.includes(value as T)) invalid(`${label} is invalid.`);
  return value as T;
}
function resolvePath(root: string, value: string): string {
  return isAbsolute(value) ? resolve(value) : resolve(root, value);
}
async function canonicalDirectory(path: string, label: string): Promise<string> {
  try {
    const canonical = await realpath(path);
    if (!(await stat(canonical)).isDirectory()) invalid(`${label} must be an existing directory.`);
    return canonical;
  } catch (error) {
    if (error instanceof ArtifactPolicyError) throw error;
    invalid(`${label} must be an existing directory.`);
  }
}
function invalid(message: string): never {
  throw new ArtifactPolicyError('WOML_ARTIFACT_POLICY_INVALID', message);
}
