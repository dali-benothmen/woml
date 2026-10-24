import { createHash } from 'node:crypto';
import { constants } from 'node:fs';
import {
  access,
  lstat,
  readFile,
  readdir,
  realpath,
  stat,
} from 'node:fs/promises';
import { dirname, extname, relative, resolve, sep } from 'node:path';

import {
  isWomlElement,
  parseWoml,
  type WomlSourceElement,
} from '@woml/compiler';

import {
  loadArtifactPolicy,
  type ArtifactRuntimeAuthorityV1,
} from './artifact-policy';
import {
  infrastructurePolicyRuntimeAuthority,
  resolveInfrastructurePolicy,
  type InfrastructurePolicyRuntimeAuthorityV1,
} from './infrastructure-policy';
import {
  loadProcessPolicy,
  type ProcessRuntimeAuthorityV1,
  type ProcessRuntimeMode,
} from './process-policy';
import { loadReviewerDirectory } from './reviewer-directory';
import {
  resolveRuntimeConfiguration,
  type ResolvedRuntimeConfiguration,
} from './runtime-config';
import {
  loadTargetRegistry,
  type PreparedTargetRegistryV1,
} from './target-registry';

export const WOML_PROJECT_PROFILE = 'woml.project/v1' as const;
export const WOML_PROJECT_VERSION = 1 as const;
export const WOML_PROJECT_SCHEMA =
  'https://woml.dev/schemas/woml-project.v1.schema.json' as const;
export const WOML_PROJECT_MAX_BYTES = 256 * 1024;
export const WOML_PROJECT_MAX_JSON_DEPTH = 16;
export const WOML_PROJECT_MAX_JSON_VALUES = 10_000;
export const WOML_PROJECT_MAX_WORKFLOW_REFERENCES = 256;
export const WOML_PROJECT_MAX_WORKFLOW_FILES = 1_024;
export const WOML_PROJECT_MAX_WORKFLOW_BYTES = 4 * 1024 * 1024;

const CONFIGURATION_MAX_BYTES = 1024 * 1024;
const CONFIGURATION_MAX_JSON_DEPTH = 64;
const CONFIGURATION_MAX_JSON_VALUES = 100_000;
const projectNamePattern = /^[a-z][a-z0-9]*(?:[._-][a-z0-9]+)*$/u;

export interface WomlProjectAuthorityV1 {
  readonly processPolicy?: string;
  readonly artifactPolicy?: string;
  readonly infrastructurePolicy?: string;
  readonly policyTrust?: string;
  readonly emergencyDeny?: string;
  readonly reviewerDirectory?: string;
}

export interface WomlProjectRemoteExecutionV1 {
  readonly targetRegistry: string;
  readonly targetTrust: string;
}

export interface WomlProjectV1 {
  readonly $schema?: typeof WOML_PROJECT_SCHEMA;
  readonly profile: typeof WOML_PROJECT_PROFILE;
  readonly version: typeof WOML_PROJECT_VERSION;
  readonly name: string;
  readonly workflows: readonly string[];
  readonly runtime?: string;
  readonly authority?: WomlProjectAuthorityV1;
  readonly remoteExecution?: WomlProjectRemoteExecutionV1;
}

export type WomlProjectReferenceKey =
  | 'runtime'
  | 'processPolicy'
  | 'artifactPolicy'
  | 'infrastructurePolicy'
  | 'policyTrust'
  | 'emergencyDeny'
  | 'reviewerDirectory'
  | 'targetRegistry'
  | 'targetTrust';

export interface ResolvedProjectReferenceV1 {
  readonly path: string;
  readonly projectPath: string;
  readonly digest: string;
}

export interface ResolvedProjectWorkflowV1
  extends ResolvedProjectReferenceV1 {
  readonly workflowIds: readonly string[];
}

export interface ResolvedWomlProjectV1 {
  readonly profile: typeof WOML_PROJECT_PROFILE;
  readonly version: typeof WOML_PROJECT_VERSION;
  readonly name: string;
  readonly manifestPath: string;
  readonly root: string;
  readonly digest: string;
  readonly manifest: WomlProjectV1;
  readonly workflows: readonly ResolvedProjectWorkflowV1[];
  readonly references: Readonly<
    Partial<Record<WomlProjectReferenceKey, ResolvedProjectReferenceV1>>
  >;
  readonly runtime?: ResolvedRuntimeConfiguration;
  readonly authorities: {
    readonly process?: ProcessRuntimeAuthorityV1;
    readonly artifacts?: ArtifactRuntimeAuthorityV1;
    readonly infrastructure?: InfrastructurePolicyRuntimeAuthorityV1;
    readonly reviewers?: Awaited<ReturnType<typeof loadReviewerDirectory>>;
    readonly targets?: PreparedTargetRegistryV1;
  };
}

export interface ResolveWomlProjectOptions {
  readonly mode?: ProcessRuntimeMode;
  readonly environment?: Readonly<Record<string, string | undefined>>;
  readonly explicitReferences?: Readonly<
    Partial<Record<WomlProjectReferenceKey, string | true>>
  >;
}

export class WomlProjectError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(`${message} Corrective action: ${projectCorrectiveAction(code)}`);
    this.name = 'WomlProjectError';
    this.code = code;
  }
}

function projectCorrectiveAction(code: string): string {
  switch (code) {
    case 'WOML_PROJECT_NOT_FOUND':
      return 'restore the referenced project file or choose an existing project path.';
    case 'WOML_PROJECT_UNSUPPORTED_VERSION':
      return 'use the woml.project/v1 profile and version 1 with this WOML release.';
    case 'WOML_PROJECT_PATH_ESCAPE':
      return 'move the referenced file beneath the project directory and use a relative forward-slash path.';
    case 'WOML_PROJECT_DUPLICATE_WORKFLOW':
      return 'remove the duplicate source reference or give every runnable workflow a unique id.';
    case 'WOML_PROJECT_OPTION_CONFLICT':
      return 'choose either the project reference or the equivalent command-line option, not both.';
    default:
      return 'repair woml.project.json according to the WOML Project v1 schema.';
  }
}

function fail(code: string, message: string): never {
  throw new WomlProjectError(code, message);
}

function record(value: unknown, label: string): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    fail('WOML_PROJECT_INVALID', `${label} must be a JSON object.`);
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
    fail(
      'WOML_PROJECT_INVALID',
      `${label} contains unknown field${unknown.length === 1 ? '' : 's'}: ${unknown.join(', ')}.`
    );
  }
  const missing = required.filter(key => !Object.hasOwn(value, key));
  if (missing.length > 0) {
    fail(
      'WOML_PROJECT_INVALID',
      `${label} is missing required field${missing.length === 1 ? '' : 's'}: ${missing.join(', ')}.`
    );
  }
}

function isWellFormedUnicode(value: string): boolean {
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code >= 0xd800 && code <= 0xdbff) {
      const next = value.charCodeAt(index + 1);
      if (next < 0xdc00 || next > 0xdfff) return false;
      index += 1;
    } else if (code >= 0xdc00 && code <= 0xdfff) {
      return false;
    }
  }
  return true;
}

class StrictProjectJsonParser {
  private position = 0;
  private values = 0;

  constructor(
    private readonly source: string,
    private readonly label = 'The project manifest',
    private readonly maximumDepth = WOML_PROJECT_MAX_JSON_DEPTH,
    private readonly maximumValues = WOML_PROJECT_MAX_JSON_VALUES
  ) {}

  parse(): unknown {
    this.skipWhitespace();
    const value = this.parseValue(0);
    this.skipWhitespace();
    if (this.position !== this.source.length) this.invalidJson();
    return value;
  }

  private parseValue(depth: number): unknown {
    this.values += 1;
    if (
      depth > this.maximumDepth ||
      this.values > this.maximumValues
    ) {
      fail(
        'WOML_PROJECT_INVALID',
        `${this.label} exceeds its JSON complexity limit.`
      );
    }
    const character = this.source[this.position];
    if (character === '{') return this.parseObject(depth + 1);
    if (character === '[') return this.parseArray(depth + 1);
    if (character === '"') return this.parseString();
    if (character === 't') return this.literal('true', true);
    if (character === 'f') return this.literal('false', false);
    if (character === 'n') return this.literal('null', null);
    return this.parseNumber();
  }

  private parseObject(depth: number): Record<string, unknown> {
    this.position += 1;
    this.skipWhitespace();
    const object = Object.create(null) as Record<string, unknown>;
    const keys = new Set<string>();
    if (this.source[this.position] === '}') {
      this.position += 1;
      return object;
    }
    while (true) {
      if (this.source[this.position] !== '"') this.invalidJson();
      const key = this.parseString();
      if (keys.has(key)) {
        fail(
          'WOML_PROJECT_INVALID',
          `${this.label} contains duplicate JSON key ${JSON.stringify(key)}.`
        );
      }
      keys.add(key);
      this.skipWhitespace();
      if (this.source[this.position] !== ':') this.invalidJson();
      this.position += 1;
      this.skipWhitespace();
      object[key] = this.parseValue(depth);
      this.skipWhitespace();
      const separator = this.source[this.position];
      if (separator === '}') {
        this.position += 1;
        return object;
      }
      if (separator !== ',') this.invalidJson();
      this.position += 1;
      this.skipWhitespace();
    }
  }

  private parseArray(depth: number): unknown[] {
    this.position += 1;
    this.skipWhitespace();
    const values: unknown[] = [];
    if (this.source[this.position] === ']') {
      this.position += 1;
      return values;
    }
    while (true) {
      values.push(this.parseValue(depth));
      this.skipWhitespace();
      const separator = this.source[this.position];
      if (separator === ']') {
        this.position += 1;
        return values;
      }
      if (separator !== ',') this.invalidJson();
      this.position += 1;
      this.skipWhitespace();
    }
  }

  private parseString(): string {
    const start = this.position;
    this.position += 1;
    while (this.position < this.source.length) {
      const code = this.source.charCodeAt(this.position);
      if (code === 0x22) {
        this.position += 1;
        try {
          const value = JSON.parse(
            this.source.slice(start, this.position)
          ) as string;
          if (!isWellFormedUnicode(value)) this.invalidJson();
          return value;
        } catch {
          this.invalidJson();
        }
      }
      if (code < 0x20) this.invalidJson();
      if (code === 0x5c) {
        this.position += 1;
        const escape = this.source[this.position];
        if (escape === 'u') {
          const hexadecimal = this.source.slice(
            this.position + 1,
            this.position + 5
          );
          if (!/^[0-9A-Fa-f]{4}$/u.test(hexadecimal)) this.invalidJson();
          this.position += 5;
          continue;
        }
        if (!'"\\/bfnrt'.includes(escape ?? '')) this.invalidJson();
      }
      this.position += 1;
    }
    this.invalidJson();
  }

  private parseNumber(): number {
    const match = this.source
      .slice(this.position)
      .match(/^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?/u);
    if (match === null) this.invalidJson();
    this.position += match[0].length;
    const value = Number(match[0]);
    if (!Number.isFinite(value)) this.invalidJson();
    return value;
  }

  private literal<T>(source: string, value: T): T {
    if (!this.source.startsWith(source, this.position)) this.invalidJson();
    this.position += source.length;
    return value;
  }

  private skipWhitespace(): void {
    while (/\s/u.test(this.source[this.position] ?? '')) this.position += 1;
  }

  private invalidJson(): never {
    fail('WOML_PROJECT_INVALID', `${this.label} is not strict JSON.`);
  }
}

function boundedProjectName(value: unknown): string {
  if (
    typeof value !== 'string' ||
    Buffer.byteLength(value) > 128 ||
    !projectNamePattern.test(value)
  ) {
    fail(
      'WOML_PROJECT_INVALID',
      'Project name must be a lowercase identifier no longer than 128 bytes.'
    );
  }
  return value;
}

function relativeReference(value: unknown, label: string): string {
  if (
    typeof value !== 'string' ||
    value.length === 0 ||
    Buffer.byteLength(value) > 4096 ||
    value.includes('\0') ||
    value.includes('\\') ||
    value.startsWith('/') ||
    /^[A-Za-z]:/u.test(value) ||
    value.startsWith('//') ||
    value.split('/').includes('..') ||
    !isWellFormedUnicode(value)
  ) {
    fail(
      'WOML_PROJECT_INVALID',
      `${label} must be a bounded relative path using forward slashes.`
    );
  }
  return value;
}

function optionalReference(
  value: unknown,
  label: string
): string | undefined {
  return value === undefined ? undefined : relativeReference(value, label);
}

export function parseWomlProject(source: string): WomlProjectV1 {
  if (Buffer.byteLength(source) > WOML_PROJECT_MAX_BYTES) {
    fail(
      'WOML_PROJECT_INVALID',
      `The project manifest exceeds ${WOML_PROJECT_MAX_BYTES} bytes.`
    );
  }
  const root = record(
    new StrictProjectJsonParser(source).parse(),
    'Project manifest'
  );
  fields(
    root,
    [
      '$schema',
      'profile',
      'version',
      'name',
      'workflows',
      'runtime',
      'authority',
      'remoteExecution',
    ],
    ['profile', 'version', 'name', 'workflows'],
    'Project manifest'
  );
  if (root.profile !== WOML_PROJECT_PROFILE || root.version !== 1) {
    fail(
      'WOML_PROJECT_UNSUPPORTED_VERSION',
      'The project manifest is not WOML Project v1.'
    );
  }
  if (root.$schema !== undefined && root.$schema !== WOML_PROJECT_SCHEMA) {
    fail(
      'WOML_PROJECT_INVALID',
      `Project $schema must be ${WOML_PROJECT_SCHEMA}.`
    );
  }
  if (
    !Array.isArray(root.workflows) ||
    root.workflows.length === 0 ||
    root.workflows.length > WOML_PROJECT_MAX_WORKFLOW_REFERENCES
  ) {
    fail(
      'WOML_PROJECT_INVALID',
      `Project workflows must contain 1 through ${WOML_PROJECT_MAX_WORKFLOW_REFERENCES} references.`
    );
  }
  const workflows = root.workflows.map((value, index) =>
    relativeReference(value, `workflows[${index}]`)
  );
  if (new Set(workflows).size !== workflows.length) {
    fail('WOML_PROJECT_INVALID', 'Project workflows contains a duplicate reference.');
  }

  let authority: WomlProjectAuthorityV1 | undefined;
  if (root.authority !== undefined) {
    const authored = record(root.authority, 'Project authority');
    fields(
      authored,
      [
        'processPolicy',
        'artifactPolicy',
        'infrastructurePolicy',
        'policyTrust',
        'emergencyDeny',
        'reviewerDirectory',
      ],
      [],
      'Project authority'
    );
    const processPolicy = optionalReference(
      authored.processPolicy,
      'authority.processPolicy'
    );
    const artifactPolicy = optionalReference(
      authored.artifactPolicy,
      'authority.artifactPolicy'
    );
    const infrastructurePolicy = optionalReference(
      authored.infrastructurePolicy,
      'authority.infrastructurePolicy'
    );
    const policyTrust = optionalReference(
      authored.policyTrust,
      'authority.policyTrust'
    );
    const emergencyDeny = optionalReference(
      authored.emergencyDeny,
      'authority.emergencyDeny'
    );
    const reviewerDirectory = optionalReference(
      authored.reviewerDirectory,
      'authority.reviewerDirectory'
    );
    authority = {
      ...(processPolicy === undefined ? {} : { processPolicy }),
      ...(artifactPolicy === undefined ? {} : { artifactPolicy }),
      ...(infrastructurePolicy === undefined ? {} : { infrastructurePolicy }),
      ...(policyTrust === undefined ? {} : { policyTrust }),
      ...(emergencyDeny === undefined ? {} : { emergencyDeny }),
      ...(reviewerDirectory === undefined ? {} : { reviewerDirectory }),
    };
    if (
      authority.infrastructurePolicy !== undefined &&
      authority.policyTrust === undefined
    ) {
      fail(
        'WOML_PROJECT_INVALID',
        'authority.policyTrust is required when authority.infrastructurePolicy is configured.'
      );
    }
    if (
      authority.policyTrust !== undefined &&
      authority.infrastructurePolicy === undefined
    ) {
      fail(
        'WOML_PROJECT_INVALID',
        'authority.infrastructurePolicy is required when authority.policyTrust is configured.'
      );
    }
    if (
      authority.emergencyDeny !== undefined &&
      authority.infrastructurePolicy === undefined
    ) {
      fail(
        'WOML_PROJECT_INVALID',
        'authority.infrastructurePolicy is required when authority.emergencyDeny is configured.'
      );
    }
  }

  let remoteExecution: WomlProjectRemoteExecutionV1 | undefined;
  if (root.remoteExecution !== undefined) {
    const authored = record(root.remoteExecution, 'Project remoteExecution');
    fields(
      authored,
      ['targetRegistry', 'targetTrust'],
      ['targetRegistry', 'targetTrust'],
      'Project remoteExecution'
    );
    remoteExecution = {
      targetRegistry: relativeReference(
        authored.targetRegistry,
        'remoteExecution.targetRegistry'
      ),
      targetTrust: relativeReference(
        authored.targetTrust,
        'remoteExecution.targetTrust'
      ),
    };
  }

  const runtime = optionalReference(root.runtime, 'runtime');
  return {
    ...(root.$schema === undefined ? {} : { $schema: WOML_PROJECT_SCHEMA }),
    profile: WOML_PROJECT_PROFILE,
    version: WOML_PROJECT_VERSION,
    name: boundedProjectName(root.name),
    workflows,
    ...(runtime === undefined ? {} : { runtime }),
    ...(authority === undefined ? {} : { authority }),
    ...(remoteExecution === undefined ? {} : { remoteExecution }),
  };
}

function manifestReference(
  manifest: WomlProjectV1,
  key: WomlProjectReferenceKey
): string | undefined {
  switch (key) {
    case 'runtime':
      return manifest.runtime;
    case 'targetRegistry':
      return manifest.remoteExecution?.targetRegistry;
    case 'targetTrust':
      return manifest.remoteExecution?.targetTrust;
    default:
      return manifest.authority?.[key];
  }
}

export function assertWomlProjectOptionCompatibility(
  manifest: WomlProjectV1,
  explicit: Readonly<Partial<Record<WomlProjectReferenceKey, string | true>>>
): void {
  for (const key of Object.keys(explicit) as WomlProjectReferenceKey[]) {
    if (explicit[key] !== undefined && manifestReference(manifest, key) !== undefined) {
      fail(
        'WOML_PROJECT_OPTION_CONFLICT',
        `Project reference ${key} conflicts with an explicit command-line option.`
      );
    }
  }
}

function canonicalJson(value: unknown): string {
  if (value === null) return 'null';
  if (typeof value === 'string' || typeof value === 'boolean') {
    return JSON.stringify(value);
  }
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) {
      fail('WOML_PROJECT_INVALID', 'Project identity contains a non-finite number.');
    }
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) {
    return `[${value.map(canonicalJson).join(',')}]`;
  }
  const object = record(value, 'Project identity value');
  return `{${Object.keys(object)
    .sort()
    .map(key => `${JSON.stringify(key)}:${canonicalJson(object[key])}`)
    .join(',')}}`;
}

function digest(value: string): string {
  return `sha256:${createHash('sha256').update(value).digest('hex')}`;
}

function digestJson(value: unknown): string {
  return digest(canonicalJson(value));
}

function portableRelativePath(root: string, path: string): string {
  return relative(root, path).split(sep).join('/').normalize('NFC');
}

function outsideRoot(root: string, path: string): boolean {
  const fromRoot = relative(root, path);
  return fromRoot === '..' || fromRoot.startsWith(`..${sep}`);
}

async function canonicalProjectReference(
  root: string,
  authored: string,
  label: string
): Promise<string> {
  const candidate = resolve(root, authored);
  const canonical = await realpath(candidate).catch(() =>
    fail(
      'WOML_PROJECT_NOT_FOUND',
      `${label} does not exist: ${JSON.stringify(authored)}.`
    )
  );
  if (outsideRoot(root, canonical)) {
    fail(
      'WOML_PROJECT_PATH_ESCAPE',
      `${label} escapes the project directory: ${JSON.stringify(authored)}.`
    );
  }
  return canonical;
}

async function resolvedJsonReference(
  root: string,
  authored: string,
  label: string
): Promise<ResolvedProjectReferenceV1> {
  if (extname(authored) !== '.json') {
    fail('WOML_PROJECT_INVALID', `${label} must reference a .json file.`);
  }
  const path = await canonicalProjectReference(root, authored, label);
  const metadata = await stat(path);
  if (!metadata.isFile() || metadata.size === 0 || metadata.size > CONFIGURATION_MAX_BYTES) {
    fail(
      'WOML_PROJECT_INVALID',
      `${label} must be a non-empty JSON file no larger than ${CONFIGURATION_MAX_BYTES} bytes.`
    );
  }
  const source = await readFile(path, 'utf8');
  const decoded = new StrictProjectJsonParser(
    source,
    label,
    CONFIGURATION_MAX_JSON_DEPTH,
    CONFIGURATION_MAX_JSON_VALUES
  ).parse();
  return {
    path,
    projectPath: portableRelativePath(root, path),
    digest: digestJson(decoded),
  };
}

function lexical(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0;
}

async function workflowFilesForReference(
  root: string,
  authored: string
): Promise<readonly string[]> {
  const path = await canonicalProjectReference(root, authored, 'Workflow reference');
  const metadata = await stat(path);
  if (metadata.isFile()) {
    if (extname(path) !== '.woml') {
      fail(
        'WOML_PROJECT_INVALID',
        `Workflow reference must use the .woml extension: ${JSON.stringify(authored)}.`
      );
    }
    return [path];
  }
  if (!metadata.isDirectory()) {
    fail(
      'WOML_PROJECT_INVALID',
      `Workflow reference must identify a .woml file or directory: ${JSON.stringify(authored)}.`
    );
  }
  const discovered: string[] = [];
  for (const entry of await readdir(path, { withFileTypes: true })) {
    if (extname(entry.name) !== '.woml') continue;
    if (!entry.isFile() && !entry.isSymbolicLink()) continue;
    const child = await canonicalProjectReference(
      root,
      portableRelativePath(root, resolve(path, entry.name)),
      'Workflow file'
    );
    if (!(await stat(child)).isFile()) {
      fail('WOML_PROJECT_INVALID', `Workflow entry ${JSON.stringify(entry.name)} is not a file.`);
    }
    discovered.push(child);
  }
  discovered.sort((left, right) =>
    lexical(portableRelativePath(root, left), portableRelativePath(root, right))
  );
  if (discovered.length === 0) {
    fail(
      'WOML_PROJECT_INVALID',
      `Workflow directory contains no direct .woml files: ${JSON.stringify(authored)}.`
    );
  }
  return discovered;
}

function workflowIds(
  path: string,
  projectPath: string,
  source: string
): readonly string[] {
  let document;
  try {
    document = parseWoml(source, { file: path });
  } catch (error) {
    const detail =
      error instanceof Error && 'diagnostic' in error
        ? ` (${String(
            (error as { diagnostic?: { code?: unknown } }).diagnostic?.code ??
              'invalid WOML'
          )})`
        : '';
    fail(
      'WOML_PROJECT_INVALID',
      `Workflow source ${JSON.stringify(projectPath)} is not valid WOML markup${detail}.`
    );
  }
  const elements =
    document.root.name === 'workflow'
      ? [document.root]
      : document.root.children.filter(
          (child): child is WomlSourceElement =>
            isWomlElement(child) && child.name === 'workflow'
        );
  return elements.map(element => element.attributes.id?.value).filter(
    (value): value is string => value !== undefined
  );
}

async function resolveProjectWorkflows(
  root: string,
  references: readonly string[]
): Promise<readonly ResolvedProjectWorkflowV1[]> {
  const paths: string[] = [];
  const seen = new Set<string>();
  for (const reference of references) {
    for (const path of await workflowFilesForReference(root, reference)) {
      const identity = process.platform === 'win32' ? path.toLowerCase() : path;
      if (seen.has(identity)) {
        fail(
          'WOML_PROJECT_DUPLICATE_WORKFLOW',
          `Workflow file ${JSON.stringify(
            portableRelativePath(root, path)
          )} is selected more than once.`
        );
      }
      seen.add(identity);
      paths.push(path);
      if (paths.length > WOML_PROJECT_MAX_WORKFLOW_FILES) {
        fail(
          'WOML_PROJECT_INVALID',
          `Project resolves more than ${WOML_PROJECT_MAX_WORKFLOW_FILES} workflow files.`
        );
      }
    }
  }
  paths.sort((left, right) =>
    lexical(portableRelativePath(root, left), portableRelativePath(root, right))
  );
  const resolved: ResolvedProjectWorkflowV1[] = [];
  const ids = new Map<string, string>();
  for (const path of paths) {
    const metadata = await stat(path);
    if (!metadata.isFile() || metadata.size > WOML_PROJECT_MAX_WORKFLOW_BYTES) {
      fail(
        'WOML_PROJECT_INVALID',
        `Workflow ${JSON.stringify(portableRelativePath(root, path))} exceeds its file boundary.`
      );
    }
    const source = await readFile(path, 'utf8');
    const normalized = source.replace(/\r\n?/gu, '\n');
    const projectPath = portableRelativePath(root, path);
    const discoveredIds = workflowIds(path, projectPath, normalized);
    for (const id of discoveredIds) {
      const previous = ids.get(id);
      if (previous !== undefined) {
        fail(
          'WOML_PROJECT_DUPLICATE_WORKFLOW',
          `Workflow id ${JSON.stringify(id)} is declared by both ${JSON.stringify(
            previous
          )} and ${JSON.stringify(projectPath)}.`
        );
      }
      ids.set(id, projectPath);
    }
    resolved.push({
      path,
      projectPath,
      digest: digest(normalized),
      workflowIds: [...discoveredIds].sort(lexical),
    });
  }
  return resolved;
}

async function secureManifest(path: string): Promise<{ path: string; source: string }> {
  const absolute = resolve(path);
  const metadata = await lstat(absolute).catch(() =>
    fail('WOML_PROJECT_NOT_FOUND', `Project manifest does not exist: ${absolute}.`)
  );
  if (
    !metadata.isFile() ||
    metadata.isSymbolicLink() ||
    metadata.size === 0 ||
    metadata.size > WOML_PROJECT_MAX_BYTES
  ) {
    fail(
      'WOML_PROJECT_INVALID',
      `Project manifest must be a non-empty regular non-symlink file no larger than ${WOML_PROJECT_MAX_BYTES} bytes.`
    );
  }
  await access(absolute, constants.R_OK).catch(() =>
    fail('WOML_PROJECT_NOT_FOUND', 'Project manifest is not readable.')
  );
  const canonical = await realpath(absolute);
  return { path: canonical, source: await readFile(canonical, 'utf8') };
}

export async function resolveWomlProject(
  manifestPath: string,
  options: ResolveWomlProjectOptions = {}
): Promise<ResolvedWomlProjectV1> {
  const projectFile = await secureManifest(manifestPath);
  const root = await realpath(dirname(projectFile.path));
  const manifest = parseWomlProject(projectFile.source);
  assertWomlProjectOptionCompatibility(
    manifest,
    options.explicitReferences ?? {}
  );
  const workflows = await resolveProjectWorkflows(root, manifest.workflows);

  const authoredReferences: Readonly<
    Partial<Record<WomlProjectReferenceKey, string>>
  > = {
    ...(manifest.runtime === undefined ? {} : { runtime: manifest.runtime }),
    ...(manifest.authority?.processPolicy === undefined
      ? {}
      : { processPolicy: manifest.authority.processPolicy }),
    ...(manifest.authority?.artifactPolicy === undefined
      ? {}
      : { artifactPolicy: manifest.authority.artifactPolicy }),
    ...(manifest.authority?.infrastructurePolicy === undefined
      ? {}
      : { infrastructurePolicy: manifest.authority.infrastructurePolicy }),
    ...(manifest.authority?.policyTrust === undefined
      ? {}
      : { policyTrust: manifest.authority.policyTrust }),
    ...(manifest.authority?.emergencyDeny === undefined
      ? {}
      : { emergencyDeny: manifest.authority.emergencyDeny }),
    ...(manifest.authority?.reviewerDirectory === undefined
      ? {}
      : { reviewerDirectory: manifest.authority.reviewerDirectory }),
    ...(manifest.remoteExecution === undefined
      ? {}
      : {
          targetRegistry: manifest.remoteExecution.targetRegistry,
          targetTrust: manifest.remoteExecution.targetTrust,
        }),
  };
  const references: Partial<
    Record<WomlProjectReferenceKey, ResolvedProjectReferenceV1>
  > = {};
  for (const key of Object.keys(authoredReferences).sort(lexical) as WomlProjectReferenceKey[]) {
    references[key] = await resolvedJsonReference(
      root,
      authoredReferences[key]!,
      `Project ${key}`
    );
  }

  const mode = options.mode ?? 'foreground';
  const runtime =
    references.runtime === undefined
      ? undefined
      : await resolveRuntimeConfiguration(
          references.runtime.path,
          {},
          options.environment ?? {},
          root
        );
  const processAuthority =
    references.processPolicy === undefined
      ? undefined
      : await loadProcessPolicy(references.processPolicy.path, mode);
  const artifactAuthority =
    references.artifactPolicy === undefined
      ? undefined
      : await loadArtifactPolicy(references.artifactPolicy.path, mode);
  const infrastructurePolicy =
    references.infrastructurePolicy === undefined
      ? undefined
      : await resolveInfrastructurePolicy(references.infrastructurePolicy.path, {
          projectRoot: root,
          trustPath: references.policyTrust!.path,
        });
  const emergencyDeny =
    references.emergencyDeny === undefined
      ? undefined
      : await resolveInfrastructurePolicy(references.emergencyDeny.path, {
          projectRoot: root,
          trustPath: references.policyTrust!.path,
          emergencyDeny: true,
        });
  const reviewerAuthority =
    references.reviewerDirectory === undefined
      ? undefined
      : await loadReviewerDirectory(references.reviewerDirectory.path);
  const targets =
    references.targetRegistry === undefined
      ? undefined
      : await loadTargetRegistry(
          references.targetRegistry.path,
          references.targetTrust!.path
        );
  const infrastructureAuthority =
    infrastructurePolicy === undefined
      ? undefined
      : infrastructurePolicyRuntimeAuthority(
          infrastructurePolicy,
          emergencyDeny
        );

  const projectDigest = digestJson({
    profile: WOML_PROJECT_PROFILE,
    version: WOML_PROJECT_VERSION,
    name: manifest.name,
    workflows: workflows.map(workflow => ({
      path: workflow.projectPath,
      digest: workflow.digest,
      workflowIds: workflow.workflowIds,
    })),
    references: Object.fromEntries(
      (Object.keys(references).sort(lexical) as WomlProjectReferenceKey[]).map(
        key => [
          key,
          {
            path: references[key]!.projectPath,
            digest: references[key]!.digest,
          },
        ]
      )
    ),
    preparedAuthority: {
      ...(infrastructurePolicy === undefined
        ? {}
        : { infrastructurePolicy: infrastructurePolicy.prepared.digest }),
      ...(emergencyDeny === undefined
        ? {}
        : { emergencyDeny: emergencyDeny.prepared.digest }),
      ...(targets === undefined
        ? {}
        : {
            targetRegistry: targets.digest,
            targetTrust: targets.trust?.digest,
          }),
    },
  });

  return {
    profile: WOML_PROJECT_PROFILE,
    version: WOML_PROJECT_VERSION,
    name: manifest.name,
    manifestPath: projectFile.path,
    root,
    digest: projectDigest,
    manifest,
    workflows,
    references,
    ...(runtime === undefined ? {} : { runtime }),
    authorities: {
      ...(processAuthority === undefined ? {} : { process: processAuthority }),
      ...(artifactAuthority === undefined ? {} : { artifacts: artifactAuthority }),
      ...(infrastructureAuthority === undefined
        ? {}
        : { infrastructure: infrastructureAuthority }),
      ...(reviewerAuthority === undefined
        ? {}
        : { reviewers: reviewerAuthority }),
      ...(targets === undefined ? {} : { targets }),
    },
  };
}
