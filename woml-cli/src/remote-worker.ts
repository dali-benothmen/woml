import { randomUUID } from 'node:crypto';
import { lstat, readFile, realpath } from 'node:fs/promises';
import { request } from 'node:https';
import { dirname, isAbsolute, resolve } from 'node:path';

import { loadProcessPolicy } from './process-policy';

const MAX_CONFIGURATION_BYTES = 256 * 1024;
const MAX_RESPONSE_BYTES = 1024 * 1024;
const IDENTIFIER = /^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$/u;

export interface WorkerConfigurationV1 {
  readonly profile: 'woml.worker/v1';
  readonly version: 1;
  readonly workerId: string;
  readonly coordinatorUrl: string;
  readonly targetIds: readonly string[];
  readonly capacity: number;
  readonly tls: {
    readonly certificateRef: string;
    readonly privateKeyRef: string;
    readonly caRef: string;
  };
  readonly processPolicy: string;
  readonly secretProvider: 'environment' | 'file' | 'external';
}

export interface PreparedWorkerConfigurationV1 {
  readonly configuration: WorkerConfigurationV1;
  readonly certificatePath: string;
  readonly privateKeyPath: string;
  readonly caPath: string;
  readonly processPolicyPath: string;
}

export interface WorkerCheckV1 {
  readonly profile: 'woml.worker-check/v1';
  readonly valid: true;
  readonly workerId: string;
  readonly coordinator: string;
  readonly targets: readonly string[];
  readonly capacity: number;
  readonly transport: 'mtls';
  readonly dispatchEnabled: false;
}

export class RemoteWorkerError extends Error {
  constructor(
    readonly code:
      | 'WOML_REMOTE_PROTOCOL_INVALID'
      | 'WOML_TARGET_INCOMPATIBLE'
      | 'WOML_TARGET_UNAVAILABLE',
    message: string
  ) {
    super(message);
    this.name = 'RemoteWorkerError';
  }
}

function fail(message: string): never {
  throw new RemoteWorkerError('WOML_REMOTE_PROTOCOL_INVALID', message);
}

function exactKeys(value: Record<string, unknown>, expected: readonly string[]): boolean {
  const keys = Object.keys(value).sort();
  return keys.length === expected.length && keys.every((key, index) => key === [...expected].sort()[index]);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function validCoordinatorUrl(value: unknown): value is string {
  if (typeof value !== 'string' || value.length > 512 || !value.startsWith('https://')) return false;
  const authority = value.slice('https://'.length);
  if (authority.length === 0 || /[/@?#\s]/u.test(authority)) return false;
  try {
    const parsed = new URL(value);
    return parsed.protocol === 'https:' && parsed.hostname.length > 0 && parsed.username === '' && parsed.password === '';
  } catch {
    return false;
  }
}

function parseConfiguration(source: string): WorkerConfigurationV1 {
  if (Buffer.byteLength(source) > MAX_CONFIGURATION_BYTES) fail('Worker configuration exceeds 256 KiB.');
  let value: unknown;
  try {
    value = JSON.parse(source);
  } catch {
    fail('Worker configuration is not valid JSON.');
  }
  if (!isRecord(value) || !exactKeys(value, ['profile', 'version', 'workerId', 'coordinatorUrl', 'targetIds', 'capacity', 'tls', 'processPolicy', 'secretProvider'])) {
    fail('Worker configuration has missing or unknown fields.');
  }
  const tls = value.tls;
  if (!isRecord(tls) || !exactKeys(tls, ['certificateRef', 'privateKeyRef', 'caRef'])) fail('Worker TLS configuration is invalid.');
  if (
    value.profile !== 'woml.worker/v1' || value.version !== 1 ||
    typeof value.workerId !== 'string' || !IDENTIFIER.test(value.workerId) || value.workerId.length > 128 ||
    !validCoordinatorUrl(value.coordinatorUrl) ||
    !Array.isArray(value.targetIds) || value.targetIds.length === 0 || value.targetIds.length > 32 ||
    !value.targetIds.every(item => typeof item === 'string' && IDENTIFIER.test(item) && item.length <= 128) ||
    new Set(value.targetIds).size !== value.targetIds.length ||
    !Number.isInteger(value.capacity) || (value.capacity as number) < 1 || (value.capacity as number) > 64 ||
    typeof tls.certificateRef !== 'string' || typeof tls.privateKeyRef !== 'string' || typeof tls.caRef !== 'string' ||
    ![tls.certificateRef, tls.privateKeyRef, tls.caRef].every(item => (item as string).startsWith('file:') && (item as string).length > 5) ||
    typeof value.processPolicy !== 'string' || value.processPolicy.length === 0 || value.processPolicy.length > 512 ||
    !['environment', 'file', 'external'].includes(value.secretProvider as string)
  ) fail('Worker configuration violates the frozen v1 contract.');
  return value as unknown as WorkerConfigurationV1;
}

async function safeFile(root: string, reference: string, privateFile: boolean): Promise<string> {
  const raw = reference.startsWith('file:') ? reference.slice(5) : reference;
  const path = isAbsolute(raw) ? raw : resolve(root, raw);
  let metadata;
  try {
    metadata = await lstat(path);
  } catch {
    throw new RemoteWorkerError('WOML_TARGET_UNAVAILABLE', 'Worker operator material is unavailable.');
  }
  if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.size === 0 || metadata.size > 1024 * 1024) fail('Worker operator material must be a bounded regular non-symlink file.');
  if (process.platform !== 'win32' && ((metadata.mode & 0o022) !== 0 || (privateFile && (metadata.mode & 0o077) !== 0))) fail('Worker operator material has unsafe permissions.');
  return await realpath(path);
}

export async function prepareWorkerConfiguration(path: string): Promise<PreparedWorkerConfigurationV1> {
  const canonicalPath = await realpath(path).catch(() => { throw new RemoteWorkerError('WOML_TARGET_UNAVAILABLE', 'Worker configuration is unavailable.'); });
  const configuration = parseConfiguration(await readFile(canonicalPath, 'utf8'));
  const root = dirname(canonicalPath);
  const processPolicyPath = await safeFile(root, configuration.processPolicy, false);
  try {
    await loadProcessPolicy(processPolicyPath, 'background');
  } catch {
    throw new RemoteWorkerError('WOML_TARGET_INCOMPATIBLE', 'Worker process authority is unavailable or incompatible.');
  }
  return {
    configuration,
    certificatePath: await safeFile(root, configuration.tls.certificateRef, false),
    privateKeyPath: await safeFile(root, configuration.tls.privateKeyRef, true),
    caPath: await safeFile(root, configuration.tls.caRef, false),
    processPolicyPath,
  };
}

export async function checkWorkerConfiguration(path: string): Promise<WorkerCheckV1> {
  const prepared = await prepareWorkerConfiguration(path);
  return {
    profile: 'woml.worker-check/v1', valid: true,
    workerId: prepared.configuration.workerId,
    coordinator: prepared.configuration.coordinatorUrl,
    targets: prepared.configuration.targetIds,
    capacity: prepared.configuration.capacity,
    transport: 'mtls', dispatchEnabled: false,
  };
}

async function postJson(prepared: PreparedWorkerConfigurationV1, route: string, payload: unknown): Promise<Record<string, unknown>> {
  const [cert, key, ca] = await Promise.all([
    readFile(prepared.certificatePath), readFile(prepared.privateKeyPath), readFile(prepared.caPath),
  ]);
  const url = new URL(route, prepared.configuration.coordinatorUrl);
  const body = Buffer.from(JSON.stringify(payload));
  return await new Promise((resolveRequest, rejectRequest) => {
    const operation = request(url, {
      method: 'POST', cert, key, ca, rejectUnauthorized: true,
      headers: { 'content-type': 'application/json', 'content-length': body.length },
      timeout: 5_000,
    }, response => {
      const chunks: Buffer[] = [];
      let size = 0;
      response.on('data', chunk => {
        size += chunk.length;
        if (size > MAX_RESPONSE_BYTES) response.destroy(new Error('oversized response'));
        else chunks.push(Buffer.from(chunk));
      });
      response.on('end', () => {
        try {
          const decoded = JSON.parse(Buffer.concat(chunks).toString('utf8')) as unknown;
          if (response.statusCode !== 200 || !isRecord(decoded)) throw new Error('coordinator rejected worker');
          resolveRequest(decoded);
        } catch {
          rejectRequest(new RemoteWorkerError('WOML_TARGET_UNAVAILABLE', 'The remote coordinator rejected the worker session.'));
        }
      });
    });
    operation.on('timeout', () => operation.destroy(new Error('timeout')));
    operation.on('error', () => rejectRequest(new RemoteWorkerError('WOML_TARGET_UNAVAILABLE', 'The remote coordinator is unavailable.')));
    operation.end(body);
  });
}

export async function startWorker(
  path: string,
  workerVersion: string,
  waitForShutdown: () => Promise<void>,
  onReady?: (session: Record<string, unknown>) => void
): Promise<void> {
  const prepared = await prepareWorkerConfiguration(path);
  const config = prepared.configuration;
  const session = await postJson(prepared, '/woml/v1/workers/session', {
    profile: 'woml.worker-protocol/v1', protocolVersion: 1,
    messageId: `hello_${randomUUID().replaceAll('-', '')}`, messageType: 'hello',
    workerId: config.workerId, sentAt: new Date().toISOString(), workerVersion,
    platform: { os: process.platform === 'win32' ? 'windows' : process.platform === 'darwin' ? 'darwin' : 'linux', arch: process.arch === 'arm64' ? 'arm64' : 'x64' },
    targets: config.targetIds, capacity: config.capacity,
    capabilities: [
      { id: 'runtime.script', version: 1 },
      { id: 'services.artifacts', version: 1 },
      { id: 'services.process', version: 2 },
    ],
  });
  if (session.profile !== 'woml.worker-session/v1' || typeof session.sessionId !== 'string' || session.dispatchEnabled !== false) fail('Coordinator returned an incompatible worker session.');
  onReady?.(session);
  let stopping = false;
  const shutdown = waitForShutdown().then(() => { stopping = true; });
  let targetIndex = 0;
  while (!stopping) {
    const target = config.targetIds[targetIndex++ % config.targetIds.length]!;
    const result = await postJson(prepared, '/woml/v1/workers/poll', {
      profile: 'woml.worker-protocol/v1', protocolVersion: 1,
      messageId: `poll_${randomUUID().replaceAll('-', '')}`, messageType: 'poll',
      workerId: config.workerId, sessionId: session.sessionId,
      sentAt: new Date().toISOString(), target, dispatchId: 'none',
    });
    if ((result.status !== 'idle' && result.status !== 'draining') || result.dispatch !== null) fail('The packaged worker received an unsupported executable dispatch.');
    await Promise.race([shutdown, Bun.sleep(1_000)]);
  }
  await shutdown;
  const closed = await postJson(prepared, '/woml/v1/workers/shutdown', {
    profile: 'woml.worker-protocol/v1', protocolVersion: 1,
    messageId: `shutdown_${randomUUID().replaceAll('-', '')}`, messageType: 'shutdown',
    workerId: config.workerId, sessionId: session.sessionId, sentAt: new Date().toISOString(),
  });
  if (closed.status !== 'closed') fail('Coordinator did not close the worker session.');
}
