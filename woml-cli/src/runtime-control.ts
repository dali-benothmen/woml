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
const DEFAULT_MAX_CONCURRENT_REQUESTS = 16;
const DEFAULT_MAX_OPERATIONS_PER_MINUTE = 120;
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
  readonly maxConcurrentRequests?: number;
  readonly maxOperationsPerMinute?: number;
  readonly operations?: RuntimeAdminOperations;
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
  const maxConcurrentRequests =
    options.maxConcurrentRequests ?? DEFAULT_MAX_CONCURRENT_REQUESTS;
  const maxOperationsPerMinute =
    options.maxOperationsPerMinute ?? DEFAULT_MAX_OPERATIONS_PER_MINUTE;
  if (
    !Number.isSafeInteger(capabilityTtlMs) ||
    capabilityTtlMs < 100 ||
    !Number.isSafeInteger(maxRequestBytes) ||
    maxRequestBytes < 256 ||
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

  const server = Bun.serve({
    hostname,
    port: options.port ?? 0,
    async fetch(request) {
      if (request.method !== 'POST' || new URL(request.url).pathname !== '/v1/control') {
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
      const authorization = request.headers.get('authorization') ?? '';
      if (!safeEqual(authorization, `Bearer ${capability}`)) {
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
          !['stop', 'list_runs', 'get_run', 'cancel_run'].includes(operation) ||
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
