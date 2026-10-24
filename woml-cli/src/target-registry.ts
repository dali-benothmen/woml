import { createHash, X509Certificate } from 'node:crypto';
import { constants } from 'node:fs';
import { access, lstat, readFile, realpath } from 'node:fs/promises';
import { dirname, relative, resolve, sep } from 'node:path';

import type {
  CompiledWorkflowDefinition,
  CompiledWorkflowNode,
} from '@woml/compiler';

import type { InfrastructurePolicyBundleV1 } from './infrastructure-policy';

export const TARGET_REGISTRY_PROFILE = 'woml.target-registry/v1' as const;
export const TARGET_TRUST_PROFILE = 'woml.target-trust/v1' as const;
export const TARGET_PREFLIGHT_PROFILE = 'woml.target-preflight/v1' as const;
export const TARGET_INSPECTION_PROFILE = 'woml.target-inspection/v1' as const;
export const TARGET_REGISTRY_MAX_BYTES = 1024 * 1024;
export const TARGET_TRUST_MAX_BYTES = 256 * 1024;

const targetPattern = /^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$/u;
const capabilityPattern = /^[a-z][a-z0-9]*(?:[.-][a-z0-9]+)*$/u;
const versionPattern = /^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$/u;

export interface TargetRegistryEntryV1 {
  readonly id: string;
  readonly kind: 'worker-pool';
  readonly allowedWorkerIdentities: readonly string[];
  readonly requiredCapabilities: readonly string[];
  readonly minimumWorkerVersion: string;
  readonly maximumInFlight: number;
}

export interface TargetRegistryV1 {
  readonly profile: typeof TARGET_REGISTRY_PROFILE;
  readonly version: 1;
  readonly targets: readonly TargetRegistryEntryV1[];
}

export interface TargetTrustIdentityV1 {
  readonly workerIdentity: string;
  readonly certificatePath: string;
}

export interface TargetTrustV1 {
  readonly profile: typeof TARGET_TRUST_PROFILE;
  readonly version: 1;
  readonly identities: readonly TargetTrustIdentityV1[];
}

export interface PreparedTargetRegistryV1 {
  readonly path: string;
  readonly digest: string;
  readonly registry: TargetRegistryV1;
  readonly trust?: {
    readonly path: string;
    readonly digest: string;
    readonly identities: Readonly<
      Record<string, { readonly certificatePath: string; readonly certificateDigest: string }>
    >;
  };
}

export interface TargetRequirementV1 {
  readonly workflowId: string;
  readonly stepId: string;
  readonly target: string;
  readonly requiredCapabilities: readonly string[];
}

export interface TargetPreflightV1 {
  readonly profile: typeof TARGET_PREFLIGHT_PROFILE;
  readonly status: 'passed';
  readonly registryPath: string;
  readonly registryDigest: string;
  readonly trustPath: string;
  readonly trustDigest: string;
  readonly targets: readonly {
    readonly id: string;
    readonly requiredCapabilities: readonly string[];
    readonly eligibleWorkers: readonly string[];
    readonly minimumWorkerVersion: string;
    readonly maximumInFlight: number;
    readonly health: {
      readonly status: 'not_observed';
      readonly reason: 'worker-transport-not-active';
    };
  }[];
}

export interface TargetInspectionV1 {
  readonly profile: typeof TARGET_INSPECTION_PROFILE;
  readonly target: string;
  readonly source: 'operator-registry';
  readonly registryPath: string;
  readonly registryDigest: string;
  readonly trust: {
    readonly status: 'verified' | 'not_configured';
    readonly path?: string;
    readonly digest?: string;
  };
  readonly kind: 'worker-pool';
  readonly requiredCapabilities: readonly string[];
  readonly allowedWorkers: readonly string[];
  readonly eligibleWorkers: readonly string[];
  readonly minimumWorkerVersion: string;
  readonly maximumInFlight: number;
  readonly health: {
    readonly status: 'not_observed';
    readonly reason: 'worker-transport-not-active';
  };
}

export class TargetRegistryError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(`${message} Corrective action: provide a valid, operator-owned target registry and trust file.`);
    this.name = 'TargetRegistryError';
    this.code = code;
  }
}

function failure(code: string, message: string): never {
  throw new TargetRegistryError(code, message);
}

function record(value: unknown, label: string): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    failure('WOML_TARGET_INVALID', `${label} must be a JSON object.`);
  }
  return value as Record<string, unknown>;
}

function fields(
  value: Record<string, unknown>,
  allowed: readonly string[],
  required: readonly string[],
  label: string
): void {
  const unknown = Object.keys(value).filter(key => !allowed.includes(key));
  if (unknown.length > 0) {
    failure('WOML_TARGET_INVALID', `${label} contains unknown field${unknown.length === 1 ? '' : 's'}: ${unknown.join(', ')}.`);
  }
  const missing = required.filter(key => !Object.hasOwn(value, key));
  if (missing.length > 0) {
    failure('WOML_TARGET_INVALID', `${label} is missing required field${missing.length === 1 ? '' : 's'}: ${missing.join(', ')}.`);
  }
}

function boundedString(
  value: unknown,
  label: string,
  pattern: RegExp,
  maximum = 128
): string {
  if (
    typeof value !== 'string' ||
    value.length === 0 ||
    Buffer.byteLength(value) > maximum ||
    !pattern.test(value)
  ) {
    failure('WOML_TARGET_INVALID', `${label} is invalid or exceeds ${maximum} bytes.`);
  }
  return value;
}

function stringSet(
  value: unknown,
  label: string,
  pattern: RegExp,
  maximum: number
): readonly string[] {
  if (!Array.isArray(value) || value.length === 0 || value.length > maximum) {
    failure('WOML_TARGET_INVALID', `${label} must contain 1 through ${maximum} values.`);
  }
  const normalized = value.map((item, index) =>
    boundedString(item, `${label}[${index}]`, pattern)
  );
  if (new Set(normalized).size !== normalized.length) {
    failure('WOML_TARGET_INVALID', `${label} contains a duplicate value.`);
  }
  return [...normalized].sort();
}

function integer(value: unknown, label: string, minimum: number, maximum: number): number {
  if (!Number.isSafeInteger(value) || Number(value) < minimum || Number(value) > maximum) {
    failure('WOML_TARGET_INVALID', `${label} must be an integer from ${minimum} through ${maximum}.`);
  }
  return Number(value);
}

function canonicalJson(value: unknown): string {
  if (value === null) return 'null';
  if (typeof value === 'string' || typeof value === 'boolean') return JSON.stringify(value);
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) failure('WOML_TARGET_INVALID', 'Target configuration contains a non-finite number.');
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(',')}]`;
  const valueRecord = record(value, 'canonical target value');
  return `{${Object.keys(valueRecord)
    .sort()
    .map(key => `${JSON.stringify(key)}:${canonicalJson(valueRecord[key])}`)
    .join(',')}}`;
}

function digest(value: unknown): string {
  return `sha256:${createHash('sha256').update(canonicalJson(value)).digest('hex')}`;
}

export function digestTargetRegistry(registry: TargetRegistryV1): string {
  return digest(registry);
}

function rejectDuplicateKeys(source: string, label: string): unknown {
  // JSON.parse is retained as the syntax authority. This scanner only rejects
  // duplicate object keys so configuration cannot depend on last-key-wins.
  const stack: Array<Set<string> | null> = [];
  let index = 0;
  let expectingKey = false;
  while (index < source.length) {
    const char = source[index]!;
    if (/\s/u.test(char)) { index += 1; continue; }
    if (char === '{') { stack.push(new Set()); expectingKey = true; index += 1; continue; }
    if (char === '[') { stack.push(null); expectingKey = false; index += 1; continue; }
    if (char === '}' || char === ']') { stack.pop(); expectingKey = false; index += 1; continue; }
    if (char === ',') { expectingKey = stack.at(-1) instanceof Set; index += 1; continue; }
    if (char === ':') { expectingKey = false; index += 1; continue; }
    if (char === '"') {
      const start = index;
      index += 1;
      while (index < source.length) {
        if (source[index] === '\\') { index += 2; continue; }
        if (source[index] === '"') { index += 1; break; }
        index += 1;
      }
      if (expectingKey) {
        const key = JSON.parse(source.slice(start, index)) as string;
        const keys = stack.at(-1);
        if (keys instanceof Set) {
          if (keys.has(key)) failure('WOML_TARGET_INVALID', `${label} contains duplicate key ${JSON.stringify(key)}.`);
          keys.add(key);
        }
      }
      continue;
    }
    index += 1;
  }
  try {
    return JSON.parse(source) as unknown;
  } catch {
    failure('WOML_TARGET_INVALID', `${label} is not valid JSON.`);
  }
}

export function parseTargetRegistry(source: string): TargetRegistryV1 {
  if (Buffer.byteLength(source) > TARGET_REGISTRY_MAX_BYTES) {
    failure('WOML_TARGET_INVALID', 'The target registry exceeds 1 MiB.');
  }
  const root = record(rejectDuplicateKeys(source, 'The target registry'), 'target registry');
  fields(root, ['profile', 'version', 'targets'], ['profile', 'version', 'targets'], 'target registry');
  if (root.profile !== TARGET_REGISTRY_PROFILE || root.version !== 1) {
    failure('WOML_TARGET_INCOMPATIBLE', 'The document is not Target Registry v1.');
  }
  if (!Array.isArray(root.targets) || root.targets.length > 256) {
    failure('WOML_TARGET_INVALID', 'target registry targets must contain at most 256 entries.');
  }
  const ids = new Set<string>();
  const targets = root.targets.map((item, index): TargetRegistryEntryV1 => {
    const target = record(item, `targets[${index}]`);
    fields(
      target,
      ['id', 'kind', 'allowedWorkerIdentities', 'requiredCapabilities', 'minimumWorkerVersion', 'maximumInFlight'],
      ['id', 'kind', 'allowedWorkerIdentities', 'requiredCapabilities', 'minimumWorkerVersion', 'maximumInFlight'],
      `targets[${index}]`
    );
    const id = boundedString(target.id, `targets[${index}].id`, targetPattern);
    if (id === 'local') failure('WOML_TARGET_INVALID', 'The built-in target "local" cannot be redefined.');
    if (ids.has(id)) failure('WOML_TARGET_INVALID', `Duplicate target id ${JSON.stringify(id)}.`);
    ids.add(id);
    if (target.kind !== 'worker-pool') {
      failure('WOML_TARGET_INCOMPATIBLE', `Target ${JSON.stringify(id)} must use kind "worker-pool".`);
    }
    const minimumWorkerVersion = boundedString(
      target.minimumWorkerVersion,
      `targets[${index}].minimumWorkerVersion`,
      versionPattern,
      64
    );
    return {
      id,
      kind: 'worker-pool',
      allowedWorkerIdentities: stringSet(target.allowedWorkerIdentities, `targets[${index}].allowedWorkerIdentities`, targetPattern, 64),
      requiredCapabilities: stringSet(target.requiredCapabilities, `targets[${index}].requiredCapabilities`, capabilityPattern, 128),
      minimumWorkerVersion,
      maximumInFlight: integer(target.maximumInFlight, `targets[${index}].maximumInFlight`, 1, 64),
    };
  });
  return { profile: TARGET_REGISTRY_PROFILE, version: 1, targets: [...targets].sort((a, b) => a.id.localeCompare(b.id)) };
}

function parseTargetTrust(source: string): TargetTrustV1 {
  if (Buffer.byteLength(source) > TARGET_TRUST_MAX_BYTES) failure('WOML_TARGET_INVALID', 'The target trust file exceeds 256 KiB.');
  const root = record(rejectDuplicateKeys(source, 'The target trust file'), 'target trust');
  fields(root, ['profile', 'version', 'identities'], ['profile', 'version', 'identities'], 'target trust');
  if (root.profile !== TARGET_TRUST_PROFILE || root.version !== 1) failure('WOML_TARGET_INCOMPATIBLE', 'The document is not Target Trust v1.');
  if (!Array.isArray(root.identities) || root.identities.length === 0 || root.identities.length > 1024) failure('WOML_TARGET_INVALID', 'target trust identities must contain 1 through 1024 entries.');
  const seen = new Set<string>();
  const identities = root.identities.map((item, index): TargetTrustIdentityV1 => {
    const identity = record(item, `identities[${index}]`);
    fields(identity, ['workerIdentity', 'certificatePath'], ['workerIdentity', 'certificatePath'], `identities[${index}]`);
    const workerIdentity = boundedString(identity.workerIdentity, `identities[${index}].workerIdentity`, targetPattern);
    if (seen.has(workerIdentity)) failure('WOML_TARGET_INVALID', `Duplicate trusted worker identity ${JSON.stringify(workerIdentity)}.`);
    seen.add(workerIdentity);
    const certificatePath = boundedString(identity.certificatePath, `identities[${index}].certificatePath`, /^[^\0]+$/u, 4096);
    return { workerIdentity, certificatePath };
  });
  return { profile: TARGET_TRUST_PROFILE, version: 1, identities: identities.sort((a, b) => a.workerIdentity.localeCompare(b.workerIdentity)) };
}

async function secureConfigurationFile(path: string, maximumBytes: number, label: string): Promise<{ path: string; source: string }> {
  const absolute = resolve(path);
  const direct = await lstat(absolute).catch(() => failure('WOML_TARGET_UNAVAILABLE', `${label} could not be read: ${absolute}`));
  if (direct.isSymbolicLink() || !direct.isFile() || direct.size > maximumBytes) failure('WOML_TARGET_INVALID', `${label} must be a regular, non-symlink file within its size limit.`);
  if (process.platform !== 'win32' && (direct.mode & 0o022) !== 0) failure('WOML_TARGET_UNAUTHORIZED', `${label} must not be writable by group or other users.`);
  await access(absolute, constants.R_OK).catch(() => failure('WOML_TARGET_UNAVAILABLE', `${label} is not readable.`));
  return { path: await realpath(absolute), source: await readFile(absolute, 'utf8') };
}

async function resolveCertificate(reference: string, trustPath: string): Promise<{ path: string; digest: string }> {
  if (reference.startsWith('/') || reference.includes('\0')) failure('WOML_TARGET_INVALID', 'Certificate references must be relative paths.');
  const trustRoot = await realpath(dirname(trustPath));
  const candidate = resolve(trustRoot, reference);
  const direct = await lstat(candidate).catch(() => failure('WOML_TARGET_UNAVAILABLE', `Trusted certificate could not be read: ${reference}`));
  if (direct.isSymbolicLink() || !direct.isFile() || direct.size === 0 || direct.size > 1024 * 1024) failure('WOML_TARGET_INVALID', `Trusted certificate ${JSON.stringify(reference)} must be a bounded regular, non-symlink file.`);
  if (process.platform !== 'win32' && (direct.mode & 0o022) !== 0) failure('WOML_TARGET_UNAUTHORIZED', `Trusted certificate ${JSON.stringify(reference)} must not be writable by group or other users.`);
  const canonical = await realpath(candidate);
  const fromRoot = relative(trustRoot, canonical);
  if (fromRoot === '..' || fromRoot.startsWith(`..${sep}`)) failure('WOML_TARGET_UNAUTHORIZED', `Trusted certificate ${JSON.stringify(reference)} escapes the trust directory.`);
  const bytes = await readFile(canonical);
  const text = bytes.toString('utf8');
  if (!text.includes('-----BEGIN CERTIFICATE-----') || !text.includes('-----END CERTIFICATE-----')) failure('WOML_TARGET_INVALID', `Trusted certificate ${JSON.stringify(reference)} is not a PEM certificate.`);
  let certificate: X509Certificate;
  try {
    certificate = new X509Certificate(bytes);
  } catch {
    failure('WOML_TARGET_INVALID', `Trusted certificate ${JSON.stringify(reference)} is not a valid X.509 certificate.`);
  }
  return {
    path: canonical,
    digest: `sha256:${createHash('sha256').update(certificate.raw).digest('hex')}`,
  };
}

export async function loadTargetRegistry(
  registryPath: string,
  trustPath?: string
): Promise<PreparedTargetRegistryV1> {
  const registryFile = await secureConfigurationFile(registryPath, TARGET_REGISTRY_MAX_BYTES, 'The target registry');
  const registry = parseTargetRegistry(registryFile.source);
  if (trustPath === undefined) return { path: registryFile.path, digest: digestTargetRegistry(registry), registry };
  const trustFile = await secureConfigurationFile(trustPath, TARGET_TRUST_MAX_BYTES, 'The target trust file');
  const trust = parseTargetTrust(trustFile.source);
  const identities: Record<string, { certificatePath: string; certificateDigest: string }> = {};
  for (const identity of trust.identities) {
    const certificate = await resolveCertificate(identity.certificatePath, trustFile.path);
    identities[identity.workerIdentity] = { certificatePath: certificate.path, certificateDigest: certificate.digest };
  }
  const trustDigest = digest({
    profile: trust.profile,
    version: trust.version,
    identities: trust.identities.map(identity => ({
      workerIdentity: identity.workerIdentity,
      certificateDigest: identities[identity.workerIdentity]!.certificateDigest,
    })),
  });
  return {
    path: registryFile.path,
    digest: digestTargetRegistry(registry),
    registry,
    trust: { path: trustFile.path, digest: trustDigest, identities },
  };
}

function workflowNodes(
  workflow: CompiledWorkflowDefinition
): readonly CompiledWorkflowNode[] {
  const nodes: CompiledWorkflowNode[] = [...workflow.graph.nodes];
  if (
    workflow.schemaVersion === 16 ||
    workflow.schemaVersion === 17 ||
    workflow.schemaVersion === 18 ||
    workflow.schemaVersion === 19 ||
    workflow.schemaVersion === 20 ||
    workflow.schemaVersion === 21
  ) {
    nodes.push(...workflow.graph.forEach.flatMap(item => item.body.nodes));
  }
  if (
    workflow.schemaVersion === 19 ||
    workflow.schemaVersion === 20 ||
    workflow.schemaVersion === 21
  ) {
    nodes.push(...workflow.graph.sagas.flatMap(saga => [
      ...saga.compensations.map(action => action.node),
      ...saga.cleanup.map(action => action.node),
    ]));
  }
  return nodes;
}

export function remoteTargetRequirements(
  workflows: readonly CompiledWorkflowDefinition[]
): readonly TargetRequirementV1[] {
  return workflows.flatMap(workflow =>
    workflowNodes(workflow)
      .filter(node => node.execution !== undefined && node.execution.target !== 'local')
      .map(node => ({
        workflowId: workflow.workflowId,
        stepId: node.id,
        target: node.execution!.target,
        requiredCapabilities: [...node.execution!.requiredCapabilities].sort(),
      }))
  );
}

function matches(values: readonly string[] | undefined, value: string): boolean {
  return values === undefined || values.includes(value);
}

function policyAllows(
  policy: InfrastructurePolicyBundleV1,
  requirement: TargetRequirementV1,
  capability: string
): boolean {
  const matching = policy.rules.filter(rule => {
    const matcher = rule.match;
    return (
      matches(matcher.targets, requirement.target) &&
      matches(matcher.workflows, requirement.workflowId) &&
      matches(matcher.capabilities, capability) &&
      matches(matcher.operations, 'dispatch') &&
      matches(matcher.effects, 'unsafe-write') &&
      matches(matcher.executionModes, 'normal') &&
      matcher.executables === undefined &&
      matcher.argumentPrefix === undefined
    );
  });
  if (matching.some(rule => rule.decision === 'deny')) return false;
  const allowed = matching.filter(rule => rule.decision === 'allow');
  if (allowed.length > 0) return allowed.some(rule => rule.requires === undefined);
  return policy.defaults['unsafe-write'] === 'allow';
}

export function inspectTarget(
  prepared: PreparedTargetRegistryV1,
  targetId: string
): TargetInspectionV1 {
  const target = prepared.registry.targets.find(candidate => candidate.id === targetId);
  if (target === undefined) failure('WOML_TARGET_UNKNOWN', `Target ${JSON.stringify(targetId)} is not present in the registry.`);
  const eligibleWorkers = target.allowedWorkerIdentities.filter(identity => prepared.trust?.identities[identity] !== undefined);
  return {
    profile: TARGET_INSPECTION_PROFILE,
    target: target.id,
    source: 'operator-registry',
    registryPath: prepared.path,
    registryDigest: prepared.digest,
    trust: prepared.trust === undefined
      ? { status: 'not_configured' }
      : { status: 'verified', path: prepared.trust.path, digest: prepared.trust.digest },
    kind: target.kind,
    requiredCapabilities: target.requiredCapabilities,
    allowedWorkers: target.allowedWorkerIdentities,
    eligibleWorkers,
    minimumWorkerVersion: target.minimumWorkerVersion,
    maximumInFlight: target.maximumInFlight,
    health: { status: 'not_observed', reason: 'worker-transport-not-active' },
  };
}

export function preflightRemoteTargets(
  workflows: readonly CompiledWorkflowDefinition[],
  prepared: PreparedTargetRegistryV1 | undefined,
  infrastructurePolicy: InfrastructurePolicyBundleV1 | undefined
): TargetPreflightV1 | undefined {
  const requirements = remoteTargetRequirements(workflows);
  if (requirements.length === 0) return undefined;
  if (prepared === undefined) failure('WOML_TARGET_UNKNOWN', 'Remote placement requires --target-registry <path>.');
  if (prepared.trust === undefined) failure('WOML_TARGET_UNAUTHORIZED', 'Remote placement requires --target-trust <path>.');
  if (infrastructurePolicy === undefined) failure('WOML_TARGET_UNAUTHORIZED', 'Remote placement requires --policy <name|path>.');
  const grouped = new Map<string, Set<string>>();
  for (const requirement of requirements) {
    const target = prepared.registry.targets.find(candidate => candidate.id === requirement.target);
    if (target === undefined) failure('WOML_TARGET_UNKNOWN', `Compiled step ${requirement.workflowId}/${requirement.stepId} selects unknown target ${JSON.stringify(requirement.target)}.`);
    const missing = requirement.requiredCapabilities.filter(capability => !target.requiredCapabilities.includes(capability));
    if (missing.length > 0) failure('WOML_TARGET_CAPABILITY_UNAVAILABLE', `Target ${JSON.stringify(target.id)} does not declare required capabilities: ${missing.join(', ')}.`);
    const eligible = target.allowedWorkerIdentities.filter(identity => prepared.trust!.identities[identity] !== undefined);
    if (eligible.length === 0) failure('WOML_TARGET_UNAUTHORIZED', `Target ${JSON.stringify(target.id)} has no worker identity anchored by the target trust file.`);
    for (const capability of requirement.requiredCapabilities) {
      if (!policyAllows(infrastructurePolicy, requirement, capability)) failure('WOML_TARGET_UNAUTHORIZED', `Infrastructure policy denies ${capability}.dispatch for target ${JSON.stringify(target.id)} in workflow ${JSON.stringify(requirement.workflowId)}.`);
    }
    const capabilities = grouped.get(target.id) ?? new Set<string>();
    for (const capability of requirement.requiredCapabilities) capabilities.add(capability);
    grouped.set(target.id, capabilities);
  }
  return {
    profile: TARGET_PREFLIGHT_PROFILE,
    status: 'passed',
    registryPath: prepared.path,
    registryDigest: prepared.digest,
    trustPath: prepared.trust.path,
    trustDigest: prepared.trust.digest,
    targets: [...grouped.entries()].sort(([a], [b]) => a.localeCompare(b)).map(([id, capabilities]) => {
      const inspection = inspectTarget(prepared, id);
      return {
        id,
        requiredCapabilities: [...capabilities].sort(),
        eligibleWorkers: inspection.eligibleWorkers,
        minimumWorkerVersion: inspection.minimumWorkerVersion,
        maximumInFlight: inspection.maximumInFlight,
        health: inspection.health,
      };
    }),
  };
}

