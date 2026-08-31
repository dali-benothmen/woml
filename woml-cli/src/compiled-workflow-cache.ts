import { createHash, randomUUID } from 'node:crypto';
import {
  mkdir,
  open,
  readFile,
  rename,
  stat,
  unlink,
} from 'node:fs/promises';
import { isAbsolute, join, relative, resolve, sep } from 'node:path';

import {
  inspectCompiledWorkflowGraph,
  type CompiledWorkflowDefinition,
  type WomlAdvisoryDiagnostic,
} from '@woml/compiler';

import type { RustRuntimeModuleArtifact } from './rust-executor';

const CACHE_PROFILE = 'woml.compiled-workflow-cache/v1' as const;
const CACHE_VERSION = 1 as const;
const MAX_CACHE_BYTES = 32 * 1024 * 1024;
const MAX_SOURCES = 4_096;
const MAX_MODULES = 256;
const DIGEST = /^sha256:[0-9a-f]{64}$/u;

export interface CompiledWorkflowCacheArtifact {
  readonly workflow: CompiledWorkflowDefinition;
  readonly definitionHash: string;
  readonly runtimeModules: readonly RustRuntimeModuleArtifact[];
  readonly sourceSnapshot: readonly {
    readonly path: string;
    readonly digest: string;
  }[];
  readonly migrationDiagnostics: readonly WomlAdvisoryDiagnostic[];
  readonly reusableEditorData?: string;
}

interface CacheEnvelope {
  readonly profile: typeof CACHE_PROFILE;
  readonly cacheVersion: typeof CACHE_VERSION;
  readonly compilerIdentity: string;
  readonly sourcePath: string;
  readonly projectRoot: string;
  readonly artifactDigest: string;
  readonly artifact: CompiledWorkflowCacheArtifact;
}

export interface CompiledWorkflowCacheOptions {
  readonly sourcePath: string;
  readonly projectRoot: string;
  readonly compilerIdentity: string;
}

function digest(value: string | Uint8Array): string {
  return `sha256:${createHash('sha256').update(value).digest('hex')}`;
}

function canonicalize(value: unknown): string {
  if (
    value === null ||
    typeof value === 'boolean' ||
    typeof value === 'string'
  ) {
    return JSON.stringify(value);
  }
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) throw new TypeError('Non-finite cache number.');
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) {
    return `[${value.map(canonicalize).join(',')}]`;
  }
  if (typeof value === 'object') {
    const record = value as Readonly<Record<string, unknown>>;
    return `{${Object.keys(record)
      .sort()
      .map(key => `${JSON.stringify(key)}:${canonicalize(record[key])}`)
      .join(',')}}`;
  }
  throw new TypeError('Compiled cache artifacts must contain strict JSON.');
}

function object(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function insideProject(path: string, projectRoot: string): boolean {
  const location = relative(projectRoot, path);
  return (
    location === '' ||
    (location !== '..' && !location.startsWith(`..${sep}`) && !isAbsolute(location))
  );
}

function sourceSnapshot(
  value: unknown,
  sourcePath: string,
  projectRoot: string
): CompiledWorkflowCacheArtifact['sourceSnapshot'] | undefined {
  if (!Array.isArray(value) || value.length < 1 || value.length > MAX_SOURCES) {
    return undefined;
  }
  const seen = new Set<string>();
  const sources: Array<{ path: string; digest: string }> = [];
  for (const item of value) {
    if (!object(item) || Object.keys(item).sort().join(',') !== 'digest,path') {
      return undefined;
    }
    if (
      typeof item.path !== 'string' ||
      resolve(item.path) !== item.path ||
      !insideProject(item.path, projectRoot) ||
      typeof item.digest !== 'string' ||
      !DIGEST.test(item.digest) ||
      seen.has(item.path)
    ) {
      return undefined;
    }
    seen.add(item.path);
    sources.push({ path: item.path, digest: item.digest });
  }
  return seen.has(sourcePath) ? sources : undefined;
}

function runtimeModules(
  value: unknown
): readonly RustRuntimeModuleArtifact[] | undefined {
  if (!Array.isArray(value) || value.length > MAX_MODULES) return undefined;
  const names = new Set<string>();
  const modules: RustRuntimeModuleArtifact[] = [];
  for (const item of value) {
    if (
      !object(item) ||
      Object.keys(item).sort().join(',') !==
        'bundle,bundleDigest,exports,name,sourceMap,sourceMapDigest' ||
      typeof item.name !== 'string' ||
      item.name.length < 1 ||
      item.name.length > 128 ||
      names.has(item.name) ||
      typeof item.bundle !== 'string' ||
      typeof item.sourceMap !== 'string' ||
      typeof item.bundleDigest !== 'string' ||
      typeof item.sourceMapDigest !== 'string' ||
      !DIGEST.test(item.bundleDigest) ||
      !DIGEST.test(item.sourceMapDigest) ||
      digest(item.bundle) !== item.bundleDigest ||
      digest(item.sourceMap) !== item.sourceMapDigest ||
      !Array.isArray(item.exports) ||
      item.exports.some(
        exported =>
          typeof exported !== 'string' ||
          exported.length < 1 ||
          exported.length > 256
      ) ||
      new Set(item.exports).size !== item.exports.length
    ) {
      return undefined;
    }
    names.add(item.name);
    modules.push({
      name: item.name,
      bundleDigest: item.bundleDigest,
      sourceMapDigest: item.sourceMapDigest,
      exports: item.exports,
      bundle: item.bundle,
      sourceMap: item.sourceMap,
    });
  }
  return modules;
}

function position(value: unknown): boolean {
  return (
    object(value) &&
    Object.keys(value).sort().join(',') === 'column,line,offset' &&
    Number.isSafeInteger(value.line) &&
    Number(value.line) >= 1 &&
    Number.isSafeInteger(value.column) &&
    Number(value.column) >= 1 &&
    Number.isSafeInteger(value.offset) &&
    Number(value.offset) >= 0
  );
}

function diagnostics(value: unknown): readonly WomlAdvisoryDiagnostic[] | undefined {
  if (!Array.isArray(value) || value.length > 4_096) return undefined;
  const decoded: WomlAdvisoryDiagnostic[] = [];
  for (const item of value) {
    if (
      !object(item) ||
      item.severity !== 'warning' ||
      typeof item.code !== 'string' ||
      typeof item.phase !== 'string' ||
      !['parse', 'validation', 'compile', 'runtime'].includes(item.phase) ||
      typeof item.message !== 'string' ||
      typeof item.file !== 'string' ||
      !object(item.location) ||
      !position(item.location.start) ||
      !position(item.location.end) ||
      (item.hint !== undefined && typeof item.hint !== 'string')
    ) {
      return undefined;
    }
    decoded.push(item as unknown as WomlAdvisoryDiagnostic);
  }
  return decoded;
}

function decodeArtifact(
  value: unknown,
  options: CompiledWorkflowCacheOptions
): CompiledWorkflowCacheArtifact | undefined {
  if (
    !object(value) ||
    !object(value.workflow) ||
    !Number.isSafeInteger(value.workflow.schemaVersion) ||
    Number(value.workflow.schemaVersion) < 1 ||
    Number(value.workflow.schemaVersion) > 16 ||
    typeof value.workflow.workflowId !== 'string' ||
    !object(value.workflow.graph) ||
    !Array.isArray(value.workflow.graph.nodes) ||
    !Array.isArray(value.workflow.graph.edges) ||
    !Array.isArray(value.workflow.graph.entryNodeIds) ||
    typeof value.definitionHash !== 'string' ||
    !DIGEST.test(value.definitionHash) ||
    (value.reusableEditorData !== undefined &&
      typeof value.reusableEditorData !== 'string')
  ) {
    return undefined;
  }
  const sources = sourceSnapshot(
    value.sourceSnapshot,
    options.sourcePath,
    options.projectRoot
  );
  const modules = runtimeModules(value.runtimeModules);
  const warnings = diagnostics(value.migrationDiagnostics);
  if (sources === undefined || modules === undefined || warnings === undefined) {
    return undefined;
  }
  const workflow = value.workflow as unknown as CompiledWorkflowDefinition;
  try {
    if (inspectCompiledWorkflowGraph(workflow.graph).length > 0) return undefined;
  } catch {
    return undefined;
  }
  return {
    workflow,
    definitionHash: value.definitionHash,
    runtimeModules: modules,
    sourceSnapshot: sources,
    migrationDiagnostics: warnings,
    ...(value.reusableEditorData === undefined
      ? {}
      : { reusableEditorData: value.reusableEditorData }),
  };
}

export function compiledWorkflowCachePath(
  options: CompiledWorkflowCacheOptions
): string {
  const sourceIdentity = createHash('sha256')
    .update(resolve(options.sourcePath))
    .digest('hex');
  return join(
    resolve(options.projectRoot),
    '.woml',
    'cache',
    'compiled-v1',
    `${sourceIdentity}.json`
  );
}

export async function readCompiledWorkflowCache(
  options: CompiledWorkflowCacheOptions
): Promise<CompiledWorkflowCacheArtifact | undefined> {
  try {
    const path = compiledWorkflowCachePath(options);
    const metadata = await stat(path);
    if (!metadata.isFile() || metadata.size < 2 || metadata.size > MAX_CACHE_BYTES) {
      return undefined;
    }
    const raw = await readFile(path, 'utf8');
    const decoded: unknown = JSON.parse(raw);
    if (
      !object(decoded) ||
      decoded.profile !== CACHE_PROFILE ||
      decoded.cacheVersion !== CACHE_VERSION ||
      decoded.compilerIdentity !== options.compilerIdentity ||
      decoded.sourcePath !== resolve(options.sourcePath) ||
      decoded.projectRoot !== resolve(options.projectRoot) ||
      typeof decoded.artifactDigest !== 'string' ||
      !DIGEST.test(decoded.artifactDigest) ||
      !Object.hasOwn(decoded, 'artifact') ||
      digest(canonicalize(decoded.artifact)) !== decoded.artifactDigest
    ) {
      return undefined;
    }
    const artifact = decodeArtifact(decoded.artifact, {
      ...options,
      sourcePath: resolve(options.sourcePath),
      projectRoot: resolve(options.projectRoot),
    });
    if (artifact === undefined) return undefined;
    for (const source of artifact.sourceSnapshot) {
      if (digest(await readFile(source.path)) !== source.digest) return undefined;
    }
    return artifact;
  } catch {
    return undefined;
  }
}

export async function writeCompiledWorkflowCache(
  options: CompiledWorkflowCacheOptions,
  artifact: CompiledWorkflowCacheArtifact
): Promise<boolean> {
  const sourcePath = resolve(options.sourcePath);
  const projectRoot = resolve(options.projectRoot);
  const path = compiledWorkflowCachePath({ ...options, sourcePath, projectRoot });
  const temporaryPath = `${path}.${process.pid}.${randomUUID()}.tmp`;
  let temporaryCreated = false;
  try {
    const normalized = decodeArtifact(artifact, {
      ...options,
      sourcePath,
      projectRoot,
    });
    if (normalized === undefined) return false;
    const envelope: CacheEnvelope = {
      profile: CACHE_PROFILE,
      cacheVersion: CACHE_VERSION,
      compilerIdentity: options.compilerIdentity,
      sourcePath,
      projectRoot,
      artifactDigest: digest(canonicalize(normalized)),
      artifact: normalized,
    };
    const content = `${JSON.stringify(envelope)}\n`;
    if (Buffer.byteLength(content) > MAX_CACHE_BYTES) return false;
    await mkdir(join(projectRoot, '.woml', 'cache', 'compiled-v1'), {
      recursive: true,
      mode: 0o700,
    });
    const file = await open(temporaryPath, 'wx', 0o600);
    temporaryCreated = true;
    try {
      await file.writeFile(content, 'utf8');
    } finally {
      await file.close();
    }
    try {
      await rename(temporaryPath, path);
    } catch (error) {
      if (
        !(error instanceof Error &&
          'code' in error &&
          (error.code === 'EEXIST' || error.code === 'EPERM'))
      ) {
        throw error;
      }
      await unlink(path).catch(() => undefined);
      await rename(temporaryPath, path);
    }
    temporaryCreated = false;
    return true;
  } catch {
    return false;
  } finally {
    if (temporaryCreated) await unlink(temporaryPath).catch(() => undefined);
  }
}
