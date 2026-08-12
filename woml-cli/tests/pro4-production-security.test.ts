import { afterEach, describe, expect, test } from 'bun:test';
import { chmod, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import {
  MountedFileSecretStore,
  ProductionSecretStore,
  type SecretMetadata,
  type SecretProvider,
  type SecretStore,
} from '../src/secrets';
import {
  readRuntimeDescriptor,
  requestRuntimeOperation,
  startRuntimeControl,
} from '../src/runtime-control';

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(
    temporaryDirectories
      .splice(0)
      .map(path => rm(path, { recursive: true, force: true }))
  );
});

async function temporaryDirectory(label: string): Promise<string> {
  const directory = await mkdtemp(join(tmpdir(), `woml-pro4-${label}-`));
  temporaryDirectories.push(directory);
  return directory;
}

class TestStore implements SecretStore {
  readonly provider: SecretProvider;
  readonly values: Readonly<Record<string, string>>;
  readonly reads: string[] = [];

  constructor(provider: SecretProvider, values: Readonly<Record<string, string>>) {
    this.provider = provider;
    this.values = values;
  }

  async get(name: string) {
    this.reads.push(name);
    return this.values[name];
  }

  async has(name: string) {
    return (await this.get(name)) !== undefined;
  }

  async list(): Promise<readonly SecretMetadata[]> {
    return Object.keys(this.values).map(name => ({ name, provider: this.provider }));
  }

  async set(): Promise<void> {
    throw new Error('read only');
  }

  async delete(): Promise<boolean> {
    throw new Error('read only');
  }
}

describe('PRO4 production secrets', () => {
  test('reads an owner-only mounted file without persisting or logging its value', async () => {
    const directory = await temporaryDirectory('mounted');
    const path = join(directory, 'DATABASE_URL');
    await writeFile(path, 'postgres://private-value\n', { mode: 0o600 });
    await chmod(path, 0o600);

    const store = new MountedFileSecretStore(directory);
    expect(await store.get('DATABASE_URL')).toBe('postgres://private-value');
    expect(await store.list()).toEqual([
      { name: 'DATABASE_URL', provider: 'mounted-files' },
    ]);
  });

  test('rejects unsafe mounted files and path traversal names', async () => {
    const directory = await temporaryDirectory('unsafe');
    const path = join(directory, 'API_TOKEN');
    await writeFile(path, 'must-not-leak', { mode: 0o644 });
    await chmod(path, 0o644);
    const store = new MountedFileSecretStore(directory);

    await expect(store.get('API_TOKEN')).rejects.toMatchObject({
      code: 'WOML_SECRET_FILE_UNSAFE',
    });
    await expect(store.get('../API_TOKEN')).rejects.toMatchObject({
      code: 'WOML_SECRET_NAME_INVALID',
    });
  });

  test('uses reviewed precedence only for requested names and rejects conflicts', async () => {
    const mounted = new TestStore('mounted-files', {
      SELECTED: 'same',
      UNUSED: 'mounted-unused',
    });
    const environment = new TestStore('environment', {
      SELECTED: 'same',
      UNUSED: 'environment-unused',
    });
    const store = new ProductionSecretStore([mounted, environment]);

    expect(await store.get('SELECTED')).toBe('same');
    expect(mounted.reads).toEqual(['SELECTED']);
    expect(environment.reads).toEqual(['SELECTED']);
    await expect(store.get('UNUSED')).rejects.toMatchObject({
      code: 'WOML_SECRET_SOURCE_CONFLICT',
    });
  });
});

describe('PRO4 authenticated runtime administration', () => {
  test('dispatches bounded live operations and never accepts public credentials', async () => {
    const calls: string[] = [];
    const control = startRuntimeControl({
      runtimeInstanceId: 'runtime_pro4_operations',
      deploymentId: 'deployment_pro4_operations',
      port: 0,
      operations: {
        listRuns: () => {
          calls.push('list');
        },
        getRun: runId => {
          calls.push(`get:${runId}`);
        },
        cancelRun: (runId, commandId) => {
          calls.push(`cancel:${runId}:${commandId}`);
          return 'WOML_RUN_OUTCOME_ALREADY_DECIDED';
        },
      },
    });
    try {
      await requestRuntimeOperation(control.descriptor, 'list_runs');
      await requestRuntimeOperation(control.descriptor, 'get_run', 'run_one');
      const cancelled = await requestRuntimeOperation(
        control.descriptor,
        'cancel_run',
        'run_one',
        fetch,
        'cancel_exactly_once'
      );
      expect(cancelled.code).toBe('WOML_RUN_OUTCOME_ALREADY_DECIDED');
      expect(calls).toEqual([
        'list',
        'get:run_one',
        'cancel:run_one:cancel_exactly_once',
      ]);

      const publicCredential = await fetch(`${control.descriptor.adminUrl}/v1/control`, {
        method: 'POST',
        headers: {
          authorization: 'Bearer EVENT_CONTROL_TOKEN',
          'content-type': 'application/json',
        },
        body: JSON.stringify({
          profile: 'woml.runtime-admin-http/v1',
          kind: 'request',
          requestId: 'request_public_confusion',
          operation: 'cancel_run',
          subjectId: 'run_one',
        }),
      });
      expect(publicCredential.status).toBe(401);
    } finally {
      await control.close();
    }
  });

  test('rotates a published capability and rejects a stolen old descriptor', async () => {
    const directory = await temporaryDirectory('rotation');
    const descriptorPath = join(directory, 'runtime.json');
    const control = startRuntimeControl({
      runtimeInstanceId: 'runtime_pro4_rotation',
      deploymentId: 'deployment_pro4_rotation',
      port: 0,
      capabilityTtlMs: 200,
      operations: { listRuns: () => {} },
    });
    try {
      await control.publishDescriptor(descriptorPath);
      const stolen = await readRuntimeDescriptor(descriptorPath);
      await Bun.sleep(140);
      const replacement = await readRuntimeDescriptor(descriptorPath);
      expect(replacement.capability).not.toBe(stolen.capability);
      expect(Date.parse(replacement.expiresAt)).toBeGreaterThan(Date.now());
      await expect(
        requestRuntimeOperation(stolen, 'list_runs')
      ).rejects.toMatchObject({ code: 'WOML_ADMIN_UNAUTHORIZED' });
      await requestRuntimeOperation(replacement, 'list_runs');
    } finally {
      await control.close();
    }
  });

  test('a replacement runtime on the same port rejects the previous instance capability', async () => {
    const first = startRuntimeControl({
      runtimeInstanceId: 'runtime_pro4_replaced',
      deploymentId: 'deployment_pro4_replaced',
      port: 0,
      operations: { listRuns: () => {} },
    });
    const stolen = first.descriptor;
    const port = Number(new URL(stolen.adminUrl).port);
    await first.close();

    const replacement = startRuntimeControl({
      runtimeInstanceId: 'runtime_pro4_replacement',
      deploymentId: 'deployment_pro4_replaced',
      port,
      operations: { listRuns: () => {} },
    });
    try {
      await expect(
        requestRuntimeOperation(stolen, 'list_runs')
      ).rejects.toMatchObject({ code: 'WOML_ADMIN_UNAUTHORIZED' });
      await requestRuntimeOperation(replacement.descriptor, 'list_runs');
    } finally {
      await replacement.close();
    }
  });

  test('rejects expired descriptors, oversized requests, and flooding', async () => {
    const control = startRuntimeControl({
      runtimeInstanceId: 'runtime_pro4_limits',
      deploymentId: 'deployment_pro4_limits',
      port: 0,
      maxRequestBytes: 256,
      maxOperationsPerMinute: 1,
      operations: { listRuns: () => {} },
    });
    try {
      await expect(
        requestRuntimeOperation(
          { ...control.descriptor, expiresAt: new Date(0).toISOString() },
          'list_runs'
        )
      ).rejects.toMatchObject({ code: 'WOML_ADMIN_CAPABILITY_EXPIRED' });

      const oversized = await fetch(`${control.descriptor.adminUrl}/v1/control`, {
        method: 'POST',
        headers: {
          authorization: `Bearer ${control.descriptor.capability}`,
          'content-type': 'application/json',
        },
        body: JSON.stringify({ payload: 'x'.repeat(512) }),
      });
      expect(oversized.status).toBe(413);

      await requestRuntimeOperation(control.descriptor, 'list_runs');
      await expect(
        requestRuntimeOperation(control.descriptor, 'list_runs')
      ).rejects.toMatchObject({ code: 'WOML_ADMIN_REQUEST_FAILED' });
    } finally {
      await control.close();
    }
  });

  test('never writes a secret value into the owner descriptor', async () => {
    const directory = await temporaryDirectory('non-leakage');
    const descriptorPath = join(directory, 'runtime.json');
    const control = startRuntimeControl({
      runtimeInstanceId: 'runtime_pro4_non_leakage',
      deploymentId: 'deployment_pro4_non_leakage',
      port: 0,
    });
    try {
      await control.publishDescriptor(descriptorPath);
      expect(await readFile(descriptorPath, 'utf8')).not.toContain(
        'provider-secret-must-never-appear'
      );
    } finally {
      await control.close();
    }
  });

  test('rejects a descriptor copied with group-readable permissions', async () => {
    if (process.platform === 'win32') return;
    const directory = await temporaryDirectory('descriptor-mode');
    const descriptorPath = join(directory, 'runtime.json');
    const control = startRuntimeControl({
      runtimeInstanceId: 'runtime_pro4_descriptor_mode',
      deploymentId: 'deployment_pro4_descriptor_mode',
      port: 0,
    });
    try {
      await control.publishDescriptor(descriptorPath);
      await chmod(descriptorPath, 0o640);
      await expect(readRuntimeDescriptor(descriptorPath)).rejects.toMatchObject({
        code: 'WOML_RUNTIME_DESCRIPTOR_UNSAFE',
      });
    } finally {
      await control.close();
    }
  });
});
