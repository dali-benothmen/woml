import { afterEach, describe, expect, test } from 'bun:test';
import { createHash, randomUUID } from 'node:crypto';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { localNativeBinaryName } from '../src/native-platform';
import {
  executeRemoteAttemptWithRust,
  prepareRemoteAttemptWithRust,
  startRemoteAttemptWithRust,
} from '../src/rust-executor';

const temporary: string[] = [];
const digest = (value: string) => `sha256:${createHash('sha256').update(value).digest('hex')}`;

afterEach(async () => {
  await Promise.all(
    temporary.splice(0).map(path => rm(path, { recursive: true, force: true }))
  );
});

function canonicalJson(value: unknown): string {
  if (value === null || typeof value === 'string' || typeof value === 'boolean')
    return JSON.stringify(value);
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) throw new Error('non-finite fixture value');
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(',')}]`;
  if (typeof value !== 'object') throw new Error('unsupported fixture value');
  const record = value as Record<string, unknown>;
  return `{${Object.keys(record)
    .sort()
    .map(key => `${JSON.stringify(key)}:${canonicalJson(record[key])}`)
    .join(',')}}`;
}

function sha256Json(value: unknown): string {
  return digest(canonicalJson(value));
}

async function dispatchFixture() {
  const model = JSON.parse(
    await readFile(
      resolve(
        import.meta.dir,
        '../../woml/tests/fixtures/remote-execution-targets/model.v21.reviewed.json'
      ),
      'utf8'
    )
  ) as { graph: { nodes: Record<string, unknown>[] } };
  const node = model.graph.nodes.find(candidate => candidate.id === 'applyRelease');
  if (node === undefined) throw new Error('reviewed remote node is missing');

  const runId = `run_${randomUUID().replaceAll('-', '')}`;
  const invocationId = `inv_${randomUUID().replaceAll('-', '')}`;
  const targetRegistryDigest = digest('packaged-target-registry');
  const dispatchIdentity = {
    domain: 'woml.remote-dispatch/v1',
    attempt: 1,
    invocationId,
    iterationIdentity: '',
    nodeId: 'applyRelease',
    runId,
    target: 'production-private',
    targetDigest: targetRegistryDigest,
  };
  const dispatch = {
    profile: 'woml.dispatch-package/v1',
    version: 1,
    dispatchId: `dispatch_${sha256Json(dispatchIdentity).slice('sha256:'.length)}`,
    runId,
    nodeId: 'applyRelease',
    attempt: 1,
    invocationId,
    target: 'production-private',
    targetRegistryDigest,
    infrastructurePolicyDigest: digest('packaged-coordinator-policy'),
    definitionPackageRootHash: digest('packaged-definition'),
    node,
    context: {
      trigger: {},
      steps: { prepare: { release: 'payments-api@42' } },
    },
    requiredCapabilities: ['runtime.script'],
    requiredSecrets: [],
    artifactGrants: [],
    deadline: new Date(Date.now() + 60_000).toISOString().replace('.000Z', 'Z'),
    packageDigest: '',
  };
  dispatch.packageDigest = sha256Json(dispatch);
  const coordinatorAuthority = {
    policyDigest: dispatch.infrastructurePolicyDigest,
    allowedCapabilities: [{ id: 'runtime.script', version: 1 }],
    allowedSecretNames: [],
  };
  return { dispatch, coordinatorAuthority };
}

describe('packaged remote execution', () => {
  test('executes one reviewed attempt through the staged native addon and replays its terminal fact', async () => {
    const root = await mkdtemp(join(tmpdir(), 'woml-packaged-remote-'));
    temporary.push(root);
    const journalPath = join(root, 'worker-journal.sqlite');
    const nativeCorePath = resolve(
      import.meta.dir,
      '../dist',
      localNativeBinaryName(process.platform, process.arch)
    );
    const scriptHostPath = resolve(import.meta.dir, '../dist/script-host.js');
    const offer = await dispatchFixture();
    const worker = {
      workerId: 'production-runner-01',
      sessionId: `session_${randomUUID().replaceAll('-', '')}`,
      capacity: 1,
      journalPath,
      authority: {
        policyDigest: digest('packaged-worker-policy'),
        allowedCapabilities: { 'runtime.script': 1 },
        allowedSecretNames: [],
      },
    };
    const nativeOptions = { nativeCorePath };
    const options = {
      nativeCorePath,
      scriptHostPath,
      bunExecutable: process.execPath,
      prestarted: true,
    };

    const prepared = prepareRemoteAttemptWithRust(
      { dispatch: offer.dispatch, coordinatorAuthority: offer.coordinatorAuthority },
      worker,
      nativeOptions
    );
    expect(prepared.status).toBe('accepted');
    startRemoteAttemptWithRust(
      journalPath,
      offer.dispatch.dispatchId,
      worker.sessionId,
      nativeOptions
    );
    const first = await executeRemoteAttemptWithRust(
      { dispatch: offer.dispatch, coordinatorAuthority: offer.coordinatorAuthority },
      worker,
      options
    );
    const replayed = prepareRemoteAttemptWithRust(
      { dispatch: offer.dispatch, coordinatorAuthority: offer.coordinatorAuthority },
      worker,
      nativeOptions
    );

    expect(first.state).toBe('succeeded');
    expect(first.outcome).toEqual({ deployed: 'payments-api@42' });
    expect(replayed).toEqual({
      profile: 'woml.remote-attempt-preparation/v1',
      status: 'terminal',
      dispatchId: offer.dispatch.dispatchId,
      terminal: first,
    });
  });
});
