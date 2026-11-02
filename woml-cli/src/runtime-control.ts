import {
  createHash,
  randomBytes,
  randomUUID,
  timingSafeEqual,
} from 'node:crypto';
import {
  chmod,
  lstat,
  mkdir,
  readFile,
  rename,
  unlink,
  writeFile,
} from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';

import type { RuntimeObservabilitySurface } from './runtime-observability';
import {
  operationalSources,
  type OperationalExecutionPhaseV1,
  type OperationalSourceV1,
  type OperationalSubjectKind,
  type RuntimeOperationalOutputSurface,
} from './operational-output';

export interface RuntimeDescriptorV1 {
  readonly profile: 'woml.runtime-descriptor/v1';
  readonly runtimeInstanceId: string;
  readonly deploymentId: string;
  readonly pid: number;
  readonly adminUrl: string;
  readonly capability: string;
  readonly createdAt: string;
  readonly expiresAt: string;
}

export interface RuntimeControlHandle {
  readonly descriptor: RuntimeDescriptorV1;
  readonly stopRequested: Promise<void>;
  publishDescriptor(path: string): Promise<void>;
  close(): Promise<void>;
}

export type RuntimeAdminOperation =
  | 'snapshot'
  | 'stream'
  | 'health'
  | 'metrics'
  | 'list_runs'
  | 'get_run'
  | 'cancel_run'
  | 'stop';

export interface RuntimeAdminOperations {
  readonly listRuns?: () => void | Promise<void>;
  readonly getRun?: (runId: string) => void | Promise<void>;
  readonly cancelRun?: (
    runId: string,
    commandId: string
  ) => string | undefined | Promise<string | undefined>;
  readonly signal?: (
    request: RuntimeSignalRequestV1
  ) => unknown | Promise<unknown>;
}

export interface RuntimeSignalRequestV1 {
  readonly signal: string;
  readonly runId: string;
  readonly waitId: string;
  readonly correlation: string;
  readonly idempotencyKey: string;
  readonly payload: unknown;
}

export interface RuntimePresentationSurface {
  readonly run: (runId: string) => unknown | Promise<unknown>;
  readonly workflow: (
    workflowId: string,
    limit: number
  ) => unknown | Promise<unknown>;
}

interface RuntimeOutputRoute {
  readonly kind: 'page' | 'progress' | 'stream' | 'observe';
  readonly subjectKind?: OperationalSubjectKind;
  readonly subject?: string;
  readonly after: number;
  readonly limit: number;
  readonly sources: ReadonlySet<OperationalSourceV1>;
  readonly phase?: OperationalExecutionPhaseV1;
}

function runtimeOutputRoute(url: URL): RuntimeOutputRoute | undefined {
  if (url.pathname === '/v1/output/observe') {
    return {
      kind: 'observe',
      after: 0,
      limit: 200,
      sources: new Set(operationalSources),
    };
  }
  const match = url.pathname.match(
    /^\/v1\/output\/(stream\/)?(runs|workflows)\/([^/]+?)(\/progress)?$/
  );
  if (match === null) return undefined;
  let subject: string;
  try {
    subject = decodeURIComponent(match[3]!);
  } catch (error) {
    throw new RuntimeControlError(
      'WOML_OUTPUT_SUBJECT_INVALID',
      'The operational-output subject is invalid.',
      { cause: error }
    );
  }
  const subjectKind = match[2] === 'runs' ? 'run' : 'workflow';
  if (
    (subjectKind === 'run' && !/^run_[A-Za-z0-9_-]{1,252}$/.test(subject)) ||
    (subjectKind === 'workflow' &&
      (!/^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$/.test(subject) ||
        subject.length > 256)) ||
    (match[4] !== undefined && subjectKind !== 'run')
  ) {
    throw new RuntimeControlError(
      'WOML_OUTPUT_SUBJECT_INVALID',
      'The operational-output subject is invalid.'
    );
  }
  const after = Number(url.searchParams.get('after') ?? '0');
  const limit = Number(url.searchParams.get('limit') ?? '200');
  if (
    !Number.isSafeInteger(after) ||
    after < 0 ||
    !Number.isSafeInteger(limit) ||
    limit < 1 ||
    limit > 1_000
  ) {
    throw new RuntimeControlError(
      'WOML_OUTPUT_CURSOR_INVALID',
      'The operational-output cursor or page limit is invalid.'
    );
  }
  const requestedSources = url.searchParams
    .getAll('source')
    .flatMap(value => value.split(','))
    .filter(Boolean);
  if (
    requestedSources.some(
      source => !operationalSources.includes(source as OperationalSourceV1)
    )
  ) {
    throw new RuntimeControlError(
      'WOML_OUTPUT_SOURCE_INVALID',
      'The operational-output source filter is invalid.'
    );
  }
  const requestedPhase = url.searchParams.get('phase');
  if (
    requestedPhase !== null &&
    requestedPhase !== 'forward' &&
    requestedPhase !== 'corrective'
  ) {
    throw new RuntimeControlError(
      'WOML_OUTPUT_PHASE_INVALID',
      'The operational-output execution-phase filter is invalid.'
    );
  }
  return {
    kind: match[1] !== undefined
      ? 'stream'
      : match[4] !== undefined
        ? 'progress'
        : 'page',
    subjectKind,
    subject,
    after,
    limit,
    sources: new Set(
      requestedSources.length === 0
        ? operationalSources
        : (requestedSources as OperationalSourceV1[])
    ),
    ...(requestedPhase === null
      ? {}
      : { phase: requestedPhase as OperationalExecutionPhaseV1 }),
  };
}

export async function queryRuntimeOperationalOutput(
  url: URL,
  output: RuntimeOperationalOutputSurface
): Promise<unknown | Response> {
  const route = runtimeOutputRoute(url);
  if (route === undefined) {
    throw new RuntimeControlError(
      'WOML_ADMIN_NOT_FOUND',
      'The operational-output route is invalid.'
    );
  }
  if (route.kind === 'observe') return await output.observe();
  const subjectKind = route.subjectKind!;
  const subject = route.subject!;
  if (route.kind === 'progress') return await output.progress(subject);
  if (route.kind === 'stream') {
    return await output.stream({
      subjectKind,
      subject,
      after: route.after,
      sources: route.sources,
      ...(route.phase === undefined ? {} : { phase: route.phase }),
    });
  }
  return await output.page({
    subjectKind,
    subject,
    after: route.after,
    limit: route.limit,
    sources: route.sources,
    ...(route.phase === undefined ? {} : { phase: route.phase }),
  });
}

export async function queryRuntimePresentation(
  url: URL,
  presentations: RuntimePresentationSurface
): Promise<unknown> {
  const runPrefix = '/v1/presentations/runs/';
  const workflowPrefix = '/v1/presentations/workflows/';
  try {
    if (url.pathname.startsWith(runPrefix)) {
      const runId = decodeURIComponent(url.pathname.slice(runPrefix.length));
      if (!/^run_[A-Za-z0-9_-]{1,252}$/.test(runId)) {
        throw new RuntimeControlError(
          'WOML_LOG_SUBJECT_INVALID',
          'The run presentation subject is invalid.'
        );
      }
      return await presentations.run(runId);
    }
    if (url.pathname.startsWith(workflowPrefix)) {
      const workflowId = decodeURIComponent(
        url.pathname.slice(workflowPrefix.length)
      );
      const limit = Number(url.searchParams.get('limit') ?? '10');
      if (
        workflowId.length < 1 ||
        workflowId.length > 256 ||
        !Number.isSafeInteger(limit) ||
        limit < 1 ||
        limit > 10
      ) {
        throw new RuntimeControlError(
          'WOML_RUN_PRESENTATION_LIMIT_INVALID',
          'A workflow presentation requires an ID and a history limit from 1 through 10.'
        );
      }
      return await presentations.workflow(workflowId, limit);
    }
  } catch (error) {
    if (error instanceof RuntimeControlError) throw error;
    if (error instanceof URIError) {
      throw new RuntimeControlError(
        'WOML_LOG_SUBJECT_INVALID',
        'The run presentation subject is invalid.',
        { cause: error }
      );
    }
    throw error;
  }
  throw new RuntimeControlError(
    'WOML_LOG_SUBJECT_INVALID',
    'The run presentation route is invalid.'
  );
}

export class RuntimeControlError extends Error {
  readonly code: string;

  constructor(code: string, message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = 'RuntimeControlError';
    this.code = code;
  }
}

const DEFAULT_CAPABILITY_TTL_MS = 60 * 60 * 1000;
const DEFAULT_MAX_REQUEST_BYTES = 16 * 1024;
const DEFAULT_MAX_RESPONSE_BYTES = 2 * 1024 * 1024;
const DEFAULT_MAX_CONCURRENT_REQUESTS = 16;
const DEFAULT_MAX_OPERATIONS_PER_MINUTE = 120;
const MAX_SIGNAL_REQUEST_BYTES = 1_048_576 + 4_096;
const LOOPBACK_HOSTS = new Set(['127.0.0.1', 'localhost', '::1']);

function safeEqual(left: string, right: string): boolean {
  const leftDigest = createHash('sha256').update(left).digest();
  const rightDigest = createHash('sha256').update(right).digest();
  return timingSafeEqual(leftDigest, rightDigest);
}

function adminUrl(hostname: string, port: number): string {
  return `http://${hostname.includes(':') ? `[${hostname}]` : hostname}:${port}`;
}

export function runtimeDescriptorPath(statePath: string): string {
  return join(dirname(resolve(statePath)), 'runtime.json');
}

export function runtimeLogPath(
  statePath: string,
  logDirectory?: string
): string {
  return join(
    logDirectory === undefined
      ? join(dirname(resolve(statePath)), 'logs')
      : resolve(logDirectory),
    'runtime.log'
  );
}

export async function writeRuntimeDescriptor(
  path: string,
  descriptor: RuntimeDescriptorV1
): Promise<void> {
  await mkdir(dirname(path), { recursive: true, mode: 0o700 });
  const temporary = `${path}.${process.pid}.${randomUUID()}.tmp`;
  await writeFile(temporary, `${JSON.stringify(descriptor)}\n`, {
    encoding: 'utf8',
    mode: 0o600,
    flag: 'wx',
  });
  await chmod(temporary, 0o600);
  await rename(temporary, path);
  await chmod(path, 0o600);
}

export async function removeRuntimeDescriptor(
  path: string,
  runtimeInstanceId: string
): Promise<void> {
  try {
    const current = await readRuntimeDescriptor(path);
    if (current.runtimeInstanceId === runtimeInstanceId) await unlink(path);
  } catch (error) {
    if (
      !(error instanceof Error && 'code' in error && error.code === 'ENOENT')
    ) {
      throw error;
    }
  }
}

function runtimeDescriptor(value: unknown): value is RuntimeDescriptorV1 {
  if (value === null || typeof value !== 'object' || Array.isArray(value))
    return false;
  const item = value as Record<string, unknown>;
  return (
    item.profile === 'woml.runtime-descriptor/v1' &&
    typeof item.runtimeInstanceId === 'string' &&
    item.runtimeInstanceId.length >= 1 &&
    item.runtimeInstanceId.length <= 320 &&
    typeof item.deploymentId === 'string' &&
    item.deploymentId.length >= 1 &&
    item.deploymentId.length <= 320 &&
    Number.isSafeInteger(item.pid) &&
    Number(item.pid) > 0 &&
    typeof item.adminUrl === 'string' &&
    /^http:\/\/(?:127\.0\.0\.1|localhost|\[::1\]):[1-9][0-9]{0,4}$/.test(
      item.adminUrl
    ) &&
    typeof item.capability === 'string' &&
    /^[A-Za-z0-9_-]{43}$/.test(item.capability) &&
    typeof item.createdAt === 'string' &&
    Number.isFinite(Date.parse(item.createdAt)) &&
    typeof item.expiresAt === 'string' &&
    Number.isFinite(Date.parse(item.expiresAt)) &&
    Date.parse(item.expiresAt) > Date.parse(item.createdAt)
  );
}

export async function readRuntimeDescriptor(
  path: string
): Promise<RuntimeDescriptorV1> {
  const entry = await lstat(path);
  if (!entry.isFile() || entry.isSymbolicLink()) {
    throw new RuntimeControlError(
      'WOML_RUNTIME_DESCRIPTOR_UNSAFE',
      'The runtime descriptor must be a regular file, not a link.'
    );
  }
  if (process.platform !== 'win32') {
    const currentUid =
      typeof process.getuid === 'function' ? process.getuid() : undefined;
    if (
      (currentUid !== undefined && entry.uid !== currentUid) ||
      (entry.mode & 0o077) !== 0
    ) {
      throw new RuntimeControlError(
        'WOML_RUNTIME_DESCRIPTOR_UNSAFE',
        'The runtime descriptor must be owned by the current user and inaccessible to group or other users.'
      );
    }
  }
  const parsed: unknown = JSON.parse(await readFile(path, 'utf8'));
  if (!runtimeDescriptor(parsed))
    throw new RuntimeControlError(
      'WOML_RUNTIME_DESCRIPTOR_INVALID',
      'The runtime descriptor is invalid.'
    );
  return parsed;
}

export function startRuntimeControl(options: {
  readonly runtimeInstanceId: string;
  readonly deploymentId: string;
  readonly host?: string;
  readonly port?: number;
  readonly capabilityTtlMs?: number;
  readonly maxRequestBytes?: number;
  readonly maxResponseBytes?: number;
  readonly maxConcurrentRequests?: number;
  readonly maxOperationsPerMinute?: number;
  readonly operations?: RuntimeAdminOperations;
  readonly observability?: RuntimeObservabilitySurface;
  readonly inspectorAnalytics?: { get(workflowId: string): Promise<unknown> };
  readonly presentations?: RuntimePresentationSurface;
  readonly output?: RuntimeOperationalOutputSurface;
  readonly healthEnabled?: boolean;
  readonly metricsEnabled?: boolean;
}): RuntimeControlHandle {
  const hostname = options.host ?? '127.0.0.1';
  if (!LOOPBACK_HOSTS.has(hostname)) {
    throw new RuntimeControlError(
      'WOML_ADMIN_HOST_UNSAFE',
      'The v1 WOML admin listener must bind to loopback.'
    );
  }
  const capabilityTtlMs = options.capabilityTtlMs ?? DEFAULT_CAPABILITY_TTL_MS;
  const maxRequestBytes = options.maxRequestBytes ?? DEFAULT_MAX_REQUEST_BYTES;
  const maxResponseBytes = options.maxResponseBytes ?? DEFAULT_MAX_RESPONSE_BYTES;
  const maxConcurrentRequests =
    options.maxConcurrentRequests ?? DEFAULT_MAX_CONCURRENT_REQUESTS;
  const maxOperationsPerMinute =
    options.maxOperationsPerMinute ?? DEFAULT_MAX_OPERATIONS_PER_MINUTE;
  if (
    !Number.isSafeInteger(capabilityTtlMs) ||
    capabilityTtlMs < 100 ||
    !Number.isSafeInteger(maxRequestBytes) ||
    maxRequestBytes < 256 ||
    !Number.isSafeInteger(maxResponseBytes) ||
    maxResponseBytes < 1024 ||
    !Number.isSafeInteger(maxConcurrentRequests) ||
    maxConcurrentRequests < 1 ||
    !Number.isSafeInteger(maxOperationsPerMinute) ||
    maxOperationsPerMinute < 1
  ) {
    throw new RuntimeControlError(
      'WOML_ADMIN_LIMIT_INVALID',
      'Runtime admin security limits are invalid.'
    );
  }
  let resolveStop!: () => void;
  const stopRequested = new Promise<void>(resolve => {
    resolveStop = resolve;
  });
  let capability = randomBytes(32).toString('base64url');
  let createdAt = new Date();
  let expiresAt = new Date(createdAt.getTime() + capabilityTtlMs);
  let draining = false;
  let closed = false;
  let inFlight = 0;
  let windowStartedAt = Date.now();
  let operationsInWindow = 0;
  let publishedDescriptorPath: string | undefined;
  let descriptorWrite = Promise.resolve();

  const response = (
    requestId: string,
    status: 'accepted' | 'succeeded' | 'failed',
    code?: string,
    httpStatus = 200
  ): Response =>
    Response.json(
      {
        profile: 'woml.runtime-admin-http/v1',
        kind: 'response',
        requestId,
        status,
        ...(code === undefined ? {} : { code }),
      },
      { status: httpStatus }
    );

  const authorized = (request: Request): boolean => {
    if (closed || Date.now() >= expiresAt.getTime()) return false;
    return safeEqual(
      request.headers.get('authorization') ?? '',
      `Bearer ${capability}`
    );
  };

  const beginOperation = (): Response | undefined => {
    const now = Date.now();
    if (now - windowStartedAt >= 60_000) {
      windowStartedAt = now;
      operationsInWindow = 0;
    }
    if (operationsInWindow >= maxOperationsPerMinute) {
      return Response.json(
        { error: { code: 'WOML_ADMIN_RATE_LIMITED' } },
        { status: 429, headers: { 'retry-after': '60' } }
      );
    }
    if (inFlight >= maxConcurrentRequests) {
      return Response.json(
        { error: { code: 'WOML_ADMIN_BUSY' } },
        { status: 503 }
      );
    }
    operationsInWindow += 1;
    inFlight += 1;
    return undefined;
  };

  const boundedJson = (value: unknown): Response => {
    const body = JSON.stringify(value);
    if (new TextEncoder().encode(body).byteLength > maxResponseBytes) {
      return Response.json(
        { error: { code: 'WOML_ADMIN_RESPONSE_TOO_LARGE' } },
        { status: 507 }
      );
    }
    return new Response(body, {
      headers: {
        'content-type': 'application/json; charset=utf-8',
        'cache-control': 'no-store',
      },
    });
  };

  const server = Bun.serve({
    hostname,
    port: options.port ?? 0,
    async fetch(request) {
      const url = new URL(request.url);
      if (request.method === 'GET' && url.pathname === '/v1/inspector/analytics') {
        if (!authorized(request)) return Response.json({ error: { code: 'WOML_ADMIN_UNAUTHORIZED' } }, { status: 401 });
        if (options.inspectorAnalytics === undefined) return Response.json({ error: { code: 'WOML_OBSERVABILITY_DISABLED' } }, { status: 404 });
        const workflow = url.searchParams.get('workflow');
        if (workflow === null || workflow.length < 1 || workflow.length > 256 ||
            [...url.searchParams.keys()].some(key => key !== 'workflow') || url.searchParams.getAll('workflow').length !== 1) {
          return Response.json({ error: { code: 'WOML_ADMIN_REQUEST_INVALID' } }, { status: 400 });
        }
        const limited = beginOperation();
        if (limited !== undefined) return limited;
        try {
          return boundedJson(await options.inspectorAnalytics.get(workflow));
        } catch {
          return Response.json({ error: { code: 'WOML_OBSERVABILITY_UNAVAILABLE' } }, { status: 503, headers: { 'cache-control': 'no-store' } });
        } finally { inFlight -= 1; }
      }
      if (request.method === 'POST' && url.pathname.startsWith('/v1/signals/')) {
        if (!authorized(request)) {
          return Response.json(
            { error: { code: 'WOML_ADMIN_UNAUTHORIZED' } },
            { status: 401 }
          );
        }
        if (options.operations?.signal === undefined) {
          return Response.json(
            { error: { code: 'WOML_SIGNAL_UNAVAILABLE' } },
            { status: 404 }
          );
        }
        const limited = beginOperation();
        if (limited !== undefined) return limited;
        try {
          const signal = decodeURIComponent(url.pathname.slice('/v1/signals/'.length));
          const contentLength = Number(request.headers.get('content-length') ?? 0);
          if (
            !/^[a-z][a-z0-9]*(?:[._-][a-z0-9]+)+$/.test(signal) ||
            signal.length > 128 ||
            !Number.isFinite(contentLength) ||
            contentLength < 0 ||
            contentLength > MAX_SIGNAL_REQUEST_BYTES
          ) {
            return Response.json(
              { error: { code: 'WOML_SIGNAL_REQUEST_INVALID' } },
              { status: contentLength > MAX_SIGNAL_REQUEST_BYTES ? 413 : 400 }
            );
          }
          const text = await request.text();
          if (new TextEncoder().encode(text).byteLength > MAX_SIGNAL_REQUEST_BYTES) {
            return Response.json(
              { error: { code: 'WOML_SIGNAL_PAYLOAD_TOO_LARGE' } },
              { status: 413 }
            );
          }
          const body: unknown = JSON.parse(text);
          if (body === null || typeof body !== 'object' || Array.isArray(body)) {
            throw new Error('invalid signal body');
          }
          const value = body as Record<string, unknown>;
          if (
            !Object.keys(value).every(key =>
              ['runId', 'waitId', 'correlation', 'idempotencyKey', 'payload'].includes(key)
            ) ||
            Object.keys(value).length !== 5 ||
            typeof value.runId !== 'string' ||
            value.runId.length < 1 ||
            value.runId.length > 256 ||
            typeof value.waitId !== 'string' ||
            value.waitId.length < 1 ||
            value.waitId.length > 128 ||
            typeof value.correlation !== 'string' ||
            value.correlation.length < 1 ||
            value.correlation.length > 256 ||
            typeof value.idempotencyKey !== 'string' ||
            value.idempotencyKey.length < 1 ||
            value.idempotencyKey.length > 256
          ) {
            throw new Error('invalid signal body');
          }
          return boundedJson(
            await options.operations.signal({
              signal,
              runId: value.runId,
              waitId: value.waitId,
              correlation: value.correlation,
              idempotencyKey: value.idempotencyKey,
              payload: value.payload,
            })
          );
        } catch (error) {
          if (error instanceof URIError || error instanceof SyntaxError) {
            return Response.json(
              { error: { code: 'WOML_SIGNAL_REQUEST_INVALID' } },
              { status: 400 }
            );
          }
          const candidate =
            error instanceof Error && 'code' in error ? String(error.code) : '';
          if (/^WOML_[A-Z0-9_]{1,123}$/.test(candidate)) {
            return Response.json({ error: { code: candidate } }, { status: 422 });
          }
          return Response.json(
            { error: { code: 'WOML_SIGNAL_REQUEST_INVALID' } },
            { status: 400 }
          );
        } finally {
          inFlight -= 1;
        }
      }
      if (
        request.method === 'GET' &&
        (url.pathname.startsWith('/v1/presentations/runs/') ||
          url.pathname.startsWith('/v1/presentations/workflows/'))
      ) {
        if (!authorized(request)) {
          return Response.json(
            { error: { code: 'WOML_ADMIN_UNAUTHORIZED' } },
            { status: 401 }
          );
        }
        if (options.presentations === undefined) {
          return Response.json(
            { error: { code: 'WOML_RUN_PRESENTATION_FAILED' } },
            { status: 404 }
          );
        }
        const limited = beginOperation();
        if (limited !== undefined) return limited;
        try {
          return boundedJson(
            await queryRuntimePresentation(url, options.presentations)
          );
        } catch (error) {
          const candidate =
            error instanceof Error && 'code' in error ? String(error.code) : '';
          const code = /^WOML_[A-Z0-9_]{1,123}$/.test(candidate)
            ? candidate
            : 'WOML_RUN_PRESENTATION_FAILED';
          return Response.json(
            { error: { code } },
            {
              status: code === 'WOML_RUN_NOT_FOUND'
                ? 404
                : code === 'WOML_LOG_SUBJECT_INVALID' ||
                    code === 'WOML_RUN_PRESENTATION_LIMIT_INVALID'
                  ? 400
                  : 503,
            }
          );
        } finally {
          inFlight -= 1;
        }
      }
      if (request.method === 'GET' && url.pathname.startsWith('/v1/output/')) {
        if (!authorized(request)) {
          return Response.json(
            { error: { code: 'WOML_ADMIN_UNAUTHORIZED' } },
            { status: 401 }
          );
        }
        if (options.output === undefined) {
          return Response.json(
            { error: { code: 'WOML_OUTPUT_API_UNAVAILABLE' } },
            { status: 404 }
          );
        }
        const limited = beginOperation();
        if (limited !== undefined) return limited;
        try {
          const value = await queryRuntimeOperationalOutput(url, options.output);
          return value instanceof Response ? value : boundedJson(value);
        } catch (error) {
          const candidate =
            error instanceof Error && 'code' in error ? String(error.code) : '';
          const code = /^WOML_[A-Z0-9_]{1,123}$/.test(candidate)
            ? candidate
            : 'WOML_OUTPUT_STORE_UNAVAILABLE';
          const status =
            code === 'WOML_OUTPUT_FOLLOWER_LIMIT'
              ? 429
              : code.endsWith('_INVALID')
                ? 400
                : code === 'WOML_OUTPUT_SUBJECT_NOT_FOUND'
                  ? 404
                  : 503;
          return Response.json(
            { error: { code } },
            { status, headers: { 'cache-control': 'no-store' } }
          );
        } finally {
          inFlight -= 1;
        }
      }
      if (
        request.method === 'GET' &&
        (url.pathname === '/livez' || url.pathname === '/readyz')
      ) {
        if (options.healthEnabled === false) {
          return Response.json(
            { error: { code: 'WOML_OBSERVABILITY_DISABLED' } },
            { status: 404 }
          );
        }
        if (options.observability === undefined) {
          return Response.json(
            {
              profile: 'woml.runtime-health/v1',
              kind: url.pathname === '/livez' ? 'liveness' : 'readiness',
              status: url.pathname === '/livez' ? 'ok' : 'unready',
            },
            { status: url.pathname === '/livez' ? 200 : 503 }
          );
        }
        const kind = url.pathname === '/livez' ? 'liveness' : 'readiness';
        const health = options.observability.minimalHealth(kind) as {
          status?: string;
        };
        return Response.json(health, {
          status: health.status === 'ok' ? 200 : 503,
          headers: { 'cache-control': 'no-store' },
        });
      }
      if (
        request.method === 'GET' &&
        ['/v1/health', '/v1/snapshot', '/v1/metrics', '/metrics', '/v1/stream'].includes(
          url.pathname
        )
      ) {
        if (!authorized(request)) {
          return Response.json(
            { error: { code: 'WOML_ADMIN_UNAUTHORIZED' } },
            { status: 401 }
          );
        }
        if (options.observability === undefined) {
          return Response.json(
            { error: { code: 'WOML_OBSERVABILITY_DISABLED' } },
            { status: 404 }
          );
        }
        if (
          (url.pathname === '/v1/health' && options.healthEnabled === false) ||
          ((url.pathname === '/v1/metrics' || url.pathname === '/metrics') &&
            options.metricsEnabled === false)
        ) {
          return Response.json(
            { error: { code: 'WOML_OBSERVABILITY_DISABLED' } },
            { status: 404 }
          );
        }
        const limited = beginOperation();
        if (limited !== undefined) return limited;
        try {
          if (url.pathname === '/v1/health') {
            return boundedJson(options.observability.detailedHealth());
          }
          if (url.pathname === '/v1/snapshot') {
            return boundedJson(await options.observability.snapshot());
          }
          if (url.pathname === '/v1/metrics') {
            return boundedJson(await options.observability.metrics());
          }
          if (url.pathname === '/metrics') {
            const body = await options.observability.prometheusMetrics();
            if (new TextEncoder().encode(body).byteLength > maxResponseBytes) {
              return Response.json(
                { error: { code: 'WOML_ADMIN_RESPONSE_TOO_LARGE' } },
                { status: 507 }
              );
            }
            return new Response(body, {
              headers: {
                'content-type': 'text/plain; version=0.0.4; charset=utf-8',
                'cache-control': 'no-store',
              },
            });
          }
          const rawAfter = url.searchParams.get('after');
          const after = rawAfter === null ? undefined : Number(rawAfter);
          if (
            after !== undefined &&
            (!Number.isSafeInteger(after) || after < 0)
          ) {
            return Response.json(
              { error: { code: 'WOML_ADMIN_REQUEST_INVALID' } },
              { status: 400 }
            );
          }
          return options.observability.stream(after);
        } catch {
          return Response.json(
            { error: { code: 'WOML_OBSERVABILITY_UNAVAILABLE' } },
            { status: 503, headers: { 'cache-control': 'no-store' } }
          );
        } finally {
          inFlight -= 1;
        }
      }
      if (request.method !== 'POST' || url.pathname !== '/v1/control') {
        return Response.json(
          { error: { code: 'WOML_ADMIN_NOT_FOUND' } },
          { status: 404 }
        );
      }
      if (closed || Date.now() >= expiresAt.getTime()) {
        return Response.json(
          { error: { code: 'WOML_ADMIN_CAPABILITY_EXPIRED' } },
          { status: 401 }
        );
      }
      if (!authorized(request)) {
        return Response.json(
          { error: { code: 'WOML_ADMIN_UNAUTHORIZED' } },
          { status: 401 }
        );
      }
      const contentLength = Number(request.headers.get('content-length') ?? 0);
      if (
        !Number.isFinite(contentLength) ||
        contentLength < 0 ||
        contentLength > maxRequestBytes
      ) {
        return Response.json(
          { error: { code: 'WOML_ADMIN_REQUEST_TOO_LARGE' } },
          { status: 413 }
        );
      }
      const limited = beginOperation();
      if (limited !== undefined) return limited;
      let body: unknown;
      try {
        const text = await request.text();
        if (new TextEncoder().encode(text).byteLength > maxRequestBytes) {
          inFlight -= 1;
          return Response.json(
            { error: { code: 'WOML_ADMIN_REQUEST_TOO_LARGE' } },
            { status: 413 }
          );
        }
        body = JSON.parse(text);
      } catch {
        inFlight -= 1;
        return Response.json(
          { error: { code: 'WOML_ADMIN_REQUEST_INVALID' } },
          { status: 400 }
        );
      } finally {
        // Operation execution below remains inside the same bounded slot.
      }
      try {
        const value = body as Record<string, unknown>;
        const keys =
          body !== null && typeof body === 'object' && !Array.isArray(body)
            ? Object.keys(value)
            : [];
        const operation = value?.operation as RuntimeAdminOperation;
        if (
          body === null ||
          typeof body !== 'object' ||
          Array.isArray(body) ||
          !keys.every(key =>
            ['profile', 'kind', 'requestId', 'operation', 'subjectId'].includes(key)
          ) ||
          value.profile !== 'woml.runtime-admin-http/v1' ||
          value.kind !== 'request' ||
          ![
            'snapshot',
            'stream',
            'health',
            'metrics',
            'stop',
            'list_runs',
            'get_run',
            'cancel_run',
          ].includes(operation) ||
          typeof value.requestId !== 'string' ||
          value.requestId.length < 1 ||
          value.requestId.length > 320 ||
          (value.subjectId !== undefined &&
            (typeof value.subjectId !== 'string' || value.subjectId.length > 320))
        ) {
          return Response.json(
            { error: { code: 'WOML_ADMIN_REQUEST_INVALID' } },
            { status: 400 }
          );
        }
        if (operation === 'stop') {
          if (value.subjectId !== options.runtimeInstanceId) {
            return Response.json(
              { error: { code: 'WOML_ADMIN_REQUEST_INVALID' } },
              { status: 400 }
            );
          }
          const alreadyDraining = draining;
          draining = true;
          queueMicrotask(resolveStop);
          return response(
            value.requestId,
            'accepted',
            alreadyDraining ? 'WOML_RUNTIME_ALREADY_DRAINING' : undefined
          );
        }
        if (['snapshot', 'stream', 'health', 'metrics'].includes(operation)) {
          if (value.subjectId !== undefined || options.observability === undefined)
            return response(value.requestId, 'failed', 'WOML_ADMIN_OPERATION_UNAVAILABLE');
          return response(value.requestId, 'succeeded');
        }
        if (operation === 'list_runs') {
          if (value.subjectId !== undefined || options.operations?.listRuns === undefined)
            return response(value.requestId, 'failed', 'WOML_ADMIN_OPERATION_UNAVAILABLE');
          await options.operations.listRuns();
          return response(value.requestId, 'succeeded');
        }
        if (typeof value.subjectId !== 'string' || value.subjectId.length === 0) {
          return Response.json(
            { error: { code: 'WOML_ADMIN_REQUEST_INVALID' } },
            { status: 400 }
          );
        }
        if (operation === 'get_run') {
          if (options.operations?.getRun === undefined)
            return response(value.requestId, 'failed', 'WOML_ADMIN_OPERATION_UNAVAILABLE');
          await options.operations.getRun(value.subjectId);
          return response(value.requestId, 'succeeded');
        }
        if (options.operations?.cancelRun === undefined)
          return response(value.requestId, 'failed', 'WOML_ADMIN_OPERATION_UNAVAILABLE');
        const code = await options.operations.cancelRun(
          value.subjectId,
          value.requestId
        );
        return response(value.requestId, 'succeeded', code);
      } catch (error) {
        const candidate =
          error instanceof Error && 'code' in error ? String(error.code) : '';
        const code = /^WOML_[A-Z0-9_]{1,123}$/.test(candidate)
          ? candidate
          : 'WOML_ADMIN_OPERATION_FAILED';
        const requestId =
          body !== null && typeof body === 'object' && 'requestId' in body &&
          typeof body.requestId === 'string'
            ? body.requestId
            : 'request_invalid';
        return response(requestId, 'failed', code);
      } finally {
        inFlight -= 1;
      }
    },
  });

  const currentDescriptor = (): RuntimeDescriptorV1 => ({
    profile: 'woml.runtime-descriptor/v1',
    runtimeInstanceId: options.runtimeInstanceId,
    deploymentId: options.deploymentId,
    pid: process.pid,
    adminUrl: adminUrl(hostname, server.port!),
    capability,
    createdAt: createdAt.toISOString(),
    expiresAt: expiresAt.toISOString(),
  });
  const rotate = (): void => {
    if (closed) return;
    capability = randomBytes(32).toString('base64url');
    createdAt = new Date();
    expiresAt = new Date(createdAt.getTime() + capabilityTtlMs);
    options.observability?.closeStreams();
    options.output?.closeStreams();
    if (publishedDescriptorPath !== undefined) {
      const path = publishedDescriptorPath;
      descriptorWrite = descriptorWrite.then(() =>
        writeRuntimeDescriptor(path, currentDescriptor())
      );
    }
    rotationTimer = setTimeout(rotate, Math.max(50, capabilityTtlMs / 2));
  };
  let rotationTimer = setTimeout(rotate, Math.max(50, capabilityTtlMs / 2));
  const handle: RuntimeControlHandle = {
    get descriptor() {
      return currentDescriptor();
    },
    stopRequested,
    publishDescriptor: async path => {
      publishedDescriptorPath = path;
      await writeRuntimeDescriptor(path, currentDescriptor());
    },
    close: async () => {
      if (closed) return;
      closed = true;
      clearTimeout(rotationTimer);
      options.observability?.closeStreams();
      options.output?.closeStreams();
      await server.stop(true);
      await descriptorWrite.catch(() => {});
      capability = '';
    },
  };
  return handle;
}

export async function requestRuntimeOperation(
  descriptor: RuntimeDescriptorV1,
  operation: RuntimeAdminOperation,
  subjectId?: string,
  fetcher: typeof fetch = globalThis.fetch,
  requestId = `request_${randomUUID().replaceAll('-', '')}`
): Promise<{ readonly status: 'accepted' | 'succeeded'; readonly code?: string; readonly requestId: string }> {
  if (Date.now() >= Date.parse(descriptor.expiresAt)) {
    throw new RuntimeControlError(
      'WOML_ADMIN_CAPABILITY_EXPIRED',
      'The runtime administration capability has expired.'
    );
  }
  const response = await fetcher(`${descriptor.adminUrl}/v1/control`, {
    method: 'POST',
    headers: {
      authorization: `Bearer ${descriptor.capability}`,
      'content-type': 'application/json',
    },
    body: JSON.stringify({
      profile: 'woml.runtime-admin-http/v1',
      kind: 'request',
      requestId,
      operation,
      ...(subjectId === undefined ? {} : { subjectId }),
    }),
  });
  if (!response.ok) {
    throw new RuntimeControlError(
      response.status === 401
        ? 'WOML_ADMIN_UNAUTHORIZED'
        : 'WOML_ADMIN_REQUEST_FAILED',
      `Runtime administration request failed with HTTP ${response.status}.`
    );
  }
  const result = (await response.json()) as Record<string, unknown>;
  if (
    result.profile !== 'woml.runtime-admin-http/v1' ||
    result.kind !== 'response' ||
    result.requestId !== requestId ||
    !['accepted', 'succeeded'].includes(String(result.status))
  ) {
    throw new RuntimeControlError(
      typeof result.code === 'string' ? result.code : 'WOML_ADMIN_OPERATION_FAILED',
      'The runtime administration operation failed.'
    );
  }
  return {
    status: result.status as 'accepted' | 'succeeded',
    ...(typeof result.code === 'string' ? { code: result.code } : {}),
    requestId,
  };
}

export async function requestRuntimeStop(
  descriptor: RuntimeDescriptorV1,
  fetcher: typeof fetch = globalThis.fetch
): Promise<'requested' | 'already_draining'> {
  const result = await requestRuntimeOperation(
    descriptor,
    'stop',
    descriptor.runtimeInstanceId,
    fetcher
  );
  return result.code === 'WOML_RUNTIME_ALREADY_DRAINING'
    ? 'already_draining'
    : 'requested';
}

export async function requestRuntimeSignal(
  descriptor: RuntimeDescriptorV1,
  request: RuntimeSignalRequestV1,
  fetcher: typeof fetch = globalThis.fetch
): Promise<unknown> {
  if (Date.now() >= Date.parse(descriptor.expiresAt)) {
    throw new RuntimeControlError(
      'WOML_ADMIN_CAPABILITY_EXPIRED',
      'The runtime administration capability has expired.'
    );
  }
  const response = await fetcher(
    `${descriptor.adminUrl}/v1/signals/${encodeURIComponent(request.signal)}`,
    {
      method: 'POST',
      headers: {
        authorization: `Bearer ${descriptor.capability}`,
        'content-type': 'application/json',
      },
      body: JSON.stringify({
        runId: request.runId,
        waitId: request.waitId,
        correlation: request.correlation,
        idempotencyKey: request.idempotencyKey,
        payload: request.payload,
      }),
    }
  );
  const result: unknown = await response.json();
  if (!response.ok) {
    const code =
      result !== null &&
      typeof result === 'object' &&
      'error' in result &&
      result.error !== null &&
      typeof result.error === 'object' &&
      'code' in result.error &&
      typeof result.error.code === 'string'
        ? result.error.code
        : 'WOML_SIGNAL_REQUEST_FAILED';
    throw new RuntimeControlError(
      code,
      `External signal request failed with HTTP ${response.status}.`
    );
  }
  return result;
}
