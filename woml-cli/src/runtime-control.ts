import { randomBytes, randomUUID } from 'node:crypto';
import {
  chmod,
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
  close(): Promise<void>;
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
    typeof item.deploymentId === 'string' &&
    Number.isSafeInteger(item.pid) &&
    typeof item.adminUrl === 'string' &&
    /^http:\/\/(?:127\.0\.0\.1|localhost|\[::1\]):[1-9][0-9]{0,4}$/.test(
      item.adminUrl
    ) &&
    typeof item.capability === 'string' &&
    item.capability.length >= 43 &&
    typeof item.createdAt === 'string' &&
    typeof item.expiresAt === 'string'
  );
}

export async function readRuntimeDescriptor(
  path: string
): Promise<RuntimeDescriptorV1> {
  const parsed: unknown = JSON.parse(await readFile(path, 'utf8'));
  if (!runtimeDescriptor(parsed))
    throw new Error('Runtime descriptor is invalid.');
  return parsed;
}

export function startRuntimeControl(options: {
  readonly runtimeInstanceId: string;
  readonly deploymentId: string;
  readonly host?: string;
  readonly port?: number;
}): RuntimeControlHandle {
  let resolveStop!: () => void;
  const stopRequested = new Promise<void>(resolve => {
    resolveStop = resolve;
  });
  const capability = randomBytes(32).toString('base64url');
  let draining = false;
  const server = Bun.serve({
    hostname: options.host ?? '127.0.0.1',
    port: options.port ?? 0,
    async fetch(request) {
      if (
        request.method !== 'POST' ||
        new URL(request.url).pathname !== '/v1/control'
      ) {
        return Response.json(
          { error: { code: 'WOML_ADMIN_NOT_FOUND' } },
          { status: 404 }
        );
      }
      if (request.headers.get('authorization') !== `Bearer ${capability}`) {
        return Response.json(
          { error: { code: 'WOML_ADMIN_UNAUTHORIZED' } },
          { status: 401 }
        );
      }
      let body: unknown;
      try {
        body = await request.json();
      } catch {
        return Response.json(
          { error: { code: 'WOML_ADMIN_REQUEST_INVALID' } },
          { status: 400 }
        );
      }
      const value = body as Record<string, unknown>;
      if (
        body === null ||
        typeof body !== 'object' ||
        Array.isArray(body) ||
        value.profile !== 'woml.runtime-admin-http/v1' ||
        value.kind !== 'request' ||
        value.operation !== 'stop' ||
        typeof value.requestId !== 'string' ||
        value.subjectId !== options.runtimeInstanceId
      ) {
        return Response.json(
          { error: { code: 'WOML_ADMIN_REQUEST_INVALID' } },
          { status: 400 }
        );
      }
      const alreadyDraining = draining;
      draining = true;
      queueMicrotask(resolveStop);
      return Response.json({
        profile: 'woml.runtime-admin-http/v1',
        kind: 'response',
        requestId: value.requestId,
        status: 'accepted',
        ...(alreadyDraining ? { code: 'WOML_RUNTIME_ALREADY_DRAINING' } : {}),
      });
    },
  });
  const now = new Date();
  const descriptor: RuntimeDescriptorV1 = {
    profile: 'woml.runtime-descriptor/v1',
    runtimeInstanceId: options.runtimeInstanceId,
    deploymentId: options.deploymentId,
    pid: process.pid,
    adminUrl: `http://${server.hostname}:${server.port}`,
    capability,
    createdAt: now.toISOString(),
    expiresAt: new Date(
      now.getTime() + 100 * 365 * 24 * 60 * 60 * 1000
    ).toISOString(),
  };
  return {
    descriptor,
    stopRequested,
    close: async () => {
      await server.stop(true);
    },
  };
}

export async function requestRuntimeStop(
  descriptor: RuntimeDescriptorV1,
  fetcher: typeof fetch = globalThis.fetch
): Promise<'requested' | 'already_draining'> {
  const requestId = `request_${randomUUID().replaceAll('-', '')}`;
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
      operation: 'stop',
      subjectId: descriptor.runtimeInstanceId,
    }),
  });
  if (!response.ok)
    throw new Error(
      `Runtime stop request failed with HTTP ${response.status}.`
    );
  const result = (await response.json()) as Record<string, unknown>;
  if (
    result.profile !== 'woml.runtime-admin-http/v1' ||
    result.kind !== 'response' ||
    result.requestId !== requestId ||
    result.status !== 'accepted'
  ) {
    throw new Error('Runtime returned an invalid stop response.');
  }
  return result.code === 'WOML_RUNTIME_ALREADY_DRAINING'
    ? 'already_draining'
    : 'requested';
}
