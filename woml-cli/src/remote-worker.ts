import { createHash, randomUUID } from 'node:crypto';
import { lstat, mkdir, readFile, realpath } from 'node:fs/promises';
import { request } from 'node:https';
import { dirname, isAbsolute, resolve } from 'node:path';

import { loadProcessPolicy, type ProcessRuntimeAuthorityV1 } from './process-policy';
import {
  executeRemoteAttemptWithRust,
  prepareRemoteAttemptWithRust,
  startRemoteAttemptWithRust,
} from './rust-executor';

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
  readonly processAuthority: ProcessRuntimeAuthorityV1;
  readonly journalPath: string;
}

export interface WorkerCheckV1 {
  readonly profile: 'woml.worker-check/v1';
  readonly valid: true;
  readonly workerId: string;
  readonly coordinator: string;
  readonly targets: readonly string[];
  readonly capacity: number;
  readonly transport: 'mtls';
  readonly dispatchEnabled: true;
}

export interface RemoteWorkerTerminalV1 {
  readonly profile: 'woml.worker-terminal/v1';
  readonly dispatchId: string;
  readonly invocationId: string;
  readonly state: 'succeeded' | 'failed' | 'cancelled';
  readonly outcome: unknown;
  readonly completedAt: string;
  readonly terminalDigest: string;
}

export interface RemoteWorkerDispatchV1 {
  readonly profile: 'woml.remote-dispatch-offer/v1';
  readonly offerId: string;
  readonly dispatch: Record<string, unknown> & {
    readonly dispatchId: string;
    readonly target: string;
  };
  readonly coordinatorAuthority: Record<string, unknown>;
  readonly expiresAt: string;
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
  let processAuthority: ProcessRuntimeAuthorityV1;
  try {
    processAuthority = await loadProcessPolicy(processPolicyPath, 'background');
  } catch {
    throw new RemoteWorkerError('WOML_TARGET_INCOMPATIBLE', 'Worker process authority is unavailable or incompatible.');
  }
  return {
    configuration,
    certificatePath: await safeFile(root, configuration.tls.certificateRef, false),
    privateKeyPath: await safeFile(root, configuration.tls.privateKeyRef, true),
    caPath: await safeFile(root, configuration.tls.caRef, false),
    processPolicyPath,
    processAuthority,
    journalPath: resolve(root, '.woml', 'worker-journal.sqlite'),
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
    transport: 'mtls', dispatchEnabled: true,
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
      { id: 'services.process', version: 2 },
    ],
  });
  if (
    session.profile !== 'woml.worker-session/v1' ||
    typeof session.sessionId !== 'string' ||
    typeof session.dispatchEnabled !== 'boolean'
  ) fail('Coordinator returned an incompatible worker session.');
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
    if (result.status === 'offered') {
      if (session.dispatchEnabled !== true)
        fail('The coordinator offered work to a worker without an executable dispatch runtime.');
      const offer = parseDispatchOffer(result.dispatch, target);
      let terminal: RemoteWorkerTerminalV1;
      const execution = await preparePackagedDispatch(offer, session.sessionId, prepared);
      if (execution.preparation.status === 'terminal') {
        terminal = execution.preparation.terminal;
      } else {
        await postDispatchFact(prepared, session.sessionId, config.workerId, target, offer.dispatch.dispatchId, 'accepted');
        startRemoteAttemptWithRust(
          prepared.journalPath,
          offer.dispatch.dispatchId,
          session.sessionId
        );
        await postDispatchFact(prepared, session.sessionId, config.workerId, target, offer.dispatch.dispatchId, 'started');
        terminal = await executeRemoteAttemptWithRust(
          offer,
          execution.worker,
          { prestarted: true }
        );
      }
      if (terminal.dispatchId !== offer.dispatch.dispatchId)
        fail('The worker terminal fact does not match the offered dispatch.');
      const succeeded = terminal.state === 'succeeded';
      const response = await postJson(prepared, '/woml/v1/workers/terminal', {
        profile: 'woml.worker-protocol/v1', protocolVersion: 1,
        messageId: `terminal_${randomUUID().replaceAll('-', '')}`, messageType: 'terminal',
        workerId: config.workerId, sessionId: session.sessionId,
        sentAt: terminal.completedAt, target, dispatchId: terminal.dispatchId,
        outcome: terminal.state,
        ...(succeeded
          ? { result: terminal.outcome }
          : {
              failure: {
                code: terminal.state === 'cancelled' ? 'WOML_RUN_CANCELLED' : 'WOML_REMOTE_ATTEMPT_FAILED',
                message: 'The authenticated remote attempt did not succeed.',
                retryable: false,
                ambiguous: false,
              },
            }),
        finalOutputSequence: 0,
        journalDigest: terminal.terminalDigest,
      });
      if (response.state !== terminal.state || response.dispatchId !== terminal.dispatchId)
        fail('The coordinator did not durably acknowledge the terminal fact.');
    } else if (
      (result.status !== 'idle' && result.status !== 'draining') ||
      result.dispatch !== null
    ) {
      fail('The coordinator returned an invalid worker poll result.');
    }
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

function canonicalJson(value: unknown): string {
  if (value === null || typeof value === 'string' || typeof value === 'boolean')
    return JSON.stringify(value);
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) fail('Worker authority contains a non-finite number.');
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(',')}]`;
  if (!isRecord(value)) fail('Worker authority contains an unsupported value.');
  return `{${Object.keys(value)
    .sort()
    .map(key => `${JSON.stringify(key)}:${canonicalJson(value[key])}`)
    .join(',')}}`;
}

async function preparePackagedDispatch(
  offer: RemoteWorkerDispatchV1,
  sessionId: string,
  prepared: PreparedWorkerConfigurationV1
): Promise<{
  readonly preparation: ReturnType<typeof prepareRemoteAttemptWithRust>;
  readonly worker: Parameters<typeof executeRemoteAttemptWithRust>[1];
}> {
  const required = offer.dispatch.requiredCapabilities;
  if (
    !Array.isArray(required) ||
    !required.every(item => typeof item === 'string')
  ) fail('The dispatch package has invalid capability requirements.');
  const localCapabilities: Readonly<Record<string, number>> = {
    'runtime.script': 1,
    'services.process': 2,
  };
  const allowedCapabilities = Object.fromEntries(
    required
      .filter(capability => Object.hasOwn(localCapabilities, capability))
      .map(capability => [capability, localCapabilities[capability]!])
  );
  if (Object.keys(allowedCapabilities).length !== required.length)
    throw new RemoteWorkerError(
      'WOML_TARGET_INCOMPATIBLE',
      'The worker does not implement every capability required by this dispatch.'
    );
  const requiredSecrets = offer.dispatch.requiredSecrets;
  if (
    !Array.isArray(requiredSecrets) ||
    !requiredSecrets.every(
      item => typeof item === 'string' && /^[A-Z][A-Z0-9_]{0,127}$/u.test(item)
    )
  ) fail('The dispatch package has invalid symbolic secret requirements.');
  if (prepared.configuration.secretProvider !== 'environment' && requiredSecrets.length > 0)
    throw new RemoteWorkerError(
      'WOML_TARGET_INCOMPATIBLE',
      'This packaged worker currently requires the environment secret provider.'
    );
  const resolvedSecrets: Record<string, string> = {};
  for (const name of requiredSecrets) {
    const value = process.env[name];
    if (value === undefined)
      throw new RemoteWorkerError(
        'WOML_TARGET_UNAVAILABLE',
        `Required worker-local secret ${name} is unavailable.`
      );
    resolvedSecrets[name] = value;
  }
  await mkdir(dirname(prepared.journalPath), { recursive: true, mode: 0o700 });
  const policyDigest = `sha256:${createHash('sha256')
    .update(canonicalJson(prepared.processAuthority.process.policy))
    .digest('hex')}`;
  const worker = {
    workerId: prepared.configuration.workerId,
    sessionId,
    capacity: prepared.configuration.capacity,
    journalPath: prepared.journalPath,
    authority: {
      policyDigest,
      allowedCapabilities,
      allowedSecretNames: requiredSecrets,
    },
    resolvedSecrets,
    runtimeCapabilities: prepared.processAuthority,
  };
  return {
    preparation: prepareRemoteAttemptWithRust(offer, worker),
    worker,
  };
}

function parseDispatchOffer(value: unknown, target: string): RemoteWorkerDispatchV1 {
  if (
    !isRecord(value) ||
    value.profile !== 'woml.remote-dispatch-offer/v1' ||
    typeof value.offerId !== 'string' ||
    typeof value.expiresAt !== 'string' ||
    !isRecord(value.dispatch) ||
    typeof value.dispatch.dispatchId !== 'string' ||
    value.dispatch.target !== target ||
    !isRecord(value.coordinatorAuthority) ||
    Number.isNaN(Date.parse(value.expiresAt)) ||
    Date.parse(value.expiresAt) <= Date.now()
  ) fail('The coordinator returned an invalid dispatch offer.');
  return value as unknown as RemoteWorkerDispatchV1;
}

async function postDispatchFact(
  prepared: PreparedWorkerConfigurationV1,
  sessionId: string,
  workerId: string,
  target: string,
  dispatchId: string,
  type: 'accepted' | 'started'
): Promise<void> {
  const response = await postJson(prepared, `/woml/v1/workers/${type}`, {
    profile: 'woml.worker-protocol/v1', protocolVersion: 1,
    messageId: `${type}_${randomUUID().replaceAll('-', '')}`, messageType: type,
    workerId, sessionId, sentAt: new Date().toISOString(), target, dispatchId,
  });
  if (response.status !== type || response.dispatchId !== dispatchId)
    fail(`The coordinator did not durably acknowledge the ${type} fact.`);
}
