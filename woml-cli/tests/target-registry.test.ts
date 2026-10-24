import { afterEach, describe, expect, test } from 'bun:test';
import { chmod, mkdir, mkdtemp, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import type { CompiledWorkflowDefinition } from '@woml/compiler';

import { runCli, type CliIo } from '../src/cli';
import {
  loadTargetRegistry,
  parseTargetRegistry,
  preflightRemoteTargets,
  TargetRegistryError,
  digestTargetRegistry,
} from '../src/target-registry';
import { parseInfrastructurePolicy } from '../src/infrastructure-policy';
import { runtimeActivationRequirements } from '../src/runtime-activation';

const modelPath = resolve(
  import.meta.dir,
  '../../woml/tests/fixtures/remote-execution-targets/model.v21.reviewed.json'
);
const reviewedRegistryPath = resolve(
  import.meta.dir,
  '../../woml/tests/fixtures/remote-execution-targets/target-registry.v1.json'
);
const temporary: string[] = [];
const TEST_CERTIFICATE = `-----BEGIN CERTIFICATE-----
MIIDFzCCAf+gAwIBAgIULzQJLdEqXGrh/VLqpNZtapZ6UhYwDQYJKoZIhvcNAQEL
BQAwGzEZMBcGA1UEAwwQd29tbC10ZXN0LXdvcmtlcjAeFw0yNjEwMjQxNjUyMTFa
Fw0zNjEwMjExNjUyMTFaMBsxGTAXBgNVBAMMEHdvbWwtdGVzdC13b3JrZXIwggEi
MA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQDb5Yt36LCmOFRawSvHzYgUrBAO
aG9O6qzOWO1AlNAcRgBXilLSIXGF/O97/XBHEi/WGjKOCacqJCp0s8msVqQd0URo
QjuniXo1OG1ge/dXV/e/yDxYIGI+iuD+hpRDkguZDwVE75ylnSjX7+F3D8IeYWVr
uz72XuEjbFHOl6YX0LuzhWxxd4ao5WV3UAwv5wdnZ35PGUbC8Ax/lk+TeXN+XDwt
gHXAArb4ktMPHyK5ouLZV2SnvqWPItgsOMBoVDZPjFDtWXxdbzb9pTdpHj8e0Dh6
ZFAwY/ie8v5yuMW0oXt9Wd5HeV3+t7SPGaqUqFYeuwxWXVs8ygtwAlOHB7I/AgMB
AAGjUzBRMB0GA1UdDgQWBBRdmkkGscbFoVY6VW9HC5DWyohJnjAfBgNVHSMEGDAW
gBRdmkkGscbFoVY6VW9HC5DWyohJnjAPBgNVHRMBAf8EBTADAQH/MA0GCSqGSIb3
DQEBCwUAA4IBAQApX564nhgowtp/TVjwr4pg/9iHLSCeDVhAsF4KWziBuuDuqDBz
9O/YJbLzJcgDND/bfz4GjLokjHbQoCQauh2EErYyYE+ZANS7nTK/iHbZL42i7nVY
R7vwehQ18MUspau5R0s/5CBZBs1jV2KUzZYGPIgcS7HIE1TGufR5TL6fErko1JGt
hSoRONuVMbqUR8JZlBbXROuXkCyI+LjH+bI0Ltjf4IIoOEPH7SsGIDfKrgWr9KZ+
0sXs0LYK1fO0LTf5RoBxpEHYeLAsF41gizfsI364xE8HgeKGkwjvR0oKxxf4XP13
W2iuw8hR3J80gpsiZcO3PDpjVUwXcsUnP82r
-----END CERTIFICATE-----
`;

afterEach(async () => {
  await Promise.all(temporary.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function fixture() {
  const root = await mkdtemp(join(tmpdir(), 'woml-target-registry-'));
  temporary.push(root);
  await mkdir(join(root, 'certs'));
  const registryPath = join(root, 'targets.json');
  const trustPath = join(root, 'trust.json');
  const policyPath = join(root, 'policy.json');
  const workflowPath = join(root, 'remote.woml');
  await writeFile(
    registryPath,
    JSON.stringify({
      profile: 'woml.target-registry/v1',
      version: 1,
      targets: [
        {
          id: 'production-private',
          kind: 'worker-pool',
          allowedWorkerIdentities: ['production-runner-02', 'production-runner-01'],
          requiredCapabilities: ['services.process', 'runtime.script'],
          minimumWorkerVersion: '1.1.0',
          maximumInFlight: 8,
        },
        {
          id: 'maintenance-runner',
          kind: 'worker-pool',
          allowedWorkerIdentities: ['maintenance-runner-01'],
          requiredCapabilities: ['services.process', 'runtime.script'],
          minimumWorkerVersion: '1.1.0',
          maximumInFlight: 2,
        },
      ],
    })
  );
  await writeFile(
    policyPath,
    JSON.stringify({
      contract: 'woml.infrastructure-policy',
      contractVersion: 1,
      id: 'remote-workers',
      version: '1',
      environment: 'production',
      defaults: { read: 'deny', 'idempotent-write': 'deny', 'unsafe-write': 'deny' },
      rules: [{
        id: 'allow-remote-dispatch',
        match: {
          capabilities: ['runtime.script'], operations: ['dispatch'], effects: ['unsafe-write'],
          workflows: ['remote-deployment'], targets: ['maintenance-runner', 'production-private'],
          executionModes: ['normal'],
        },
        decision: 'allow',
      }],
    })
  );
  await writeFile(
    workflowPath,
    `<woml><workflow id="remote-deployment"><triggers><manual id="start" /></triggers><steps><step id="deploy" target="production-private"><script>return { ok: true };</script></step></steps></workflow></woml>`
  );
  await writeFile(
    trustPath,
    JSON.stringify({
      profile: 'woml.target-trust/v1',
      version: 1,
      identities: [
        { workerIdentity: 'production-runner-01', certificatePath: 'certs/production.pem' },
        { workerIdentity: 'maintenance-runner-01', certificatePath: 'certs/maintenance.pem' },
      ],
    })
  );
  for (const name of ['production.pem', 'maintenance.pem']) {
    await writeFile(join(root, 'certs', name), TEST_CERTIFICATE);
  }
  if (process.platform !== 'win32') {
    await chmod(registryPath, 0o600);
    await chmod(trustPath, 0o600);
    await chmod(policyPath, 0o600);
    await chmod(join(root, 'certs', 'production.pem'), 0o644);
    await chmod(join(root, 'certs', 'maintenance.pem'), 0o644);
  }
  return { root, registryPath, trustPath, policyPath, workflowPath };
}

function allowPolicy() {
  return parseInfrastructurePolicy(JSON.stringify({
    contract: 'woml.infrastructure-policy',
    contractVersion: 1,
    id: 'remote-workers',
    version: '1',
    environment: 'production',
    defaults: { read: 'deny', 'idempotent-write': 'deny', 'unsafe-write': 'deny' },
    rules: [
      {
        id: 'allow-remote-dispatch',
        match: {
          capabilities: ['runtime.script', 'services.process'],
          operations: ['dispatch'],
          effects: ['unsafe-write'],
          workflows: ['remote-deployment'],
          targets: ['maintenance-runner', 'production-private'],
          executionModes: ['normal'],
        },
        decision: 'allow',
      },
    ],
  })).bundle;
}

async function invoke(args: readonly string[]) {
  let stdout = '';
  let stderr = '';
  const io: CliIo = {
    stdout: text => { stdout += text; },
    stderr: text => { stderr += text; },
  };
  return { exitCode: await runCli(args, io), stdout, stderr };
}

describe('Target Registry v1', () => {
  test('canonicalizes order and produces a stable digest with resolved public trust', async () => {
    const paths = await fixture();
    const first = await loadTargetRegistry(paths.registryPath, paths.trustPath);
    const second = await loadTargetRegistry(paths.registryPath, paths.trustPath);
    expect(first.digest).toBe(second.digest);
    const reviewed = parseTargetRegistry(await Bun.file(reviewedRegistryPath).text());
    expect(digestTargetRegistry(reviewed)).toBe(
      'sha256:d7f8a876583ef92e40aabaca6ed0a78f4189bbeaa374caeca1b5ed4ae7039f93'
    );
    expect(first.registry.targets.map(target => target.id)).toEqual([
      'maintenance-runner',
      'production-private',
    ]);
    expect(first.registry.targets[1]!.allowedWorkerIdentities).toEqual([
      'production-runner-01',
      'production-runner-02',
    ]);
    expect(first.trust?.identities['production-runner-01']?.certificateDigest).toMatch(/^sha256:[a-f0-9]{64}$/);
    expect(JSON.stringify(first)).not.toContain('BEGIN CERTIFICATE');
  });

  test('rejects duplicate fields, reserved local, and duplicate target ids', () => {
    expect(() => parseTargetRegistry('{"profile":"woml.target-registry/v1","profile":"woml.target-registry/v1","version":1,"targets":[]}')).toThrow(TargetRegistryError);
    expect(() => parseTargetRegistry(JSON.stringify({
      profile: 'woml.target-registry/v1', version: 1, targets: [{
        id: 'local', kind: 'worker-pool', allowedWorkerIdentities: ['worker-one'],
        requiredCapabilities: ['runtime.script'], minimumWorkerVersion: '1.0.0', maximumInFlight: 1,
      }],
    }))).toThrow(/cannot be redefined/);
  });

  test('joins compiled requirements, trust, capabilities, and infrastructure policy', async () => {
    const paths = await fixture();
    const prepared = await loadTargetRegistry(paths.registryPath, paths.trustPath);
    const model = (await Bun.file(modelPath).json()) as CompiledWorkflowDefinition;
    const result = preflightRemoteTargets([model], prepared, allowPolicy());
    expect(result?.status).toBe('passed');
    expect(result?.targets.map(target => target.id)).toEqual([
      'maintenance-runner',
      'production-private',
    ]);
    expect(result?.targets[1]!.eligibleWorkers).toEqual(['production-runner-01']);
    expect(runtimeActivationRequirements([model]).remoteExecution).toEqual({
      targets: ['maintenance-runner', 'production-private'],
      requiredCapabilities: ['runtime.script'],
    });
  });

  test('fails closed for missing trust, missing capabilities, policy denial, and symlinks', async () => {
    const paths = await fixture();
    const model = (await Bun.file(modelPath).json()) as CompiledWorkflowDefinition;
    const untrusted = await loadTargetRegistry(paths.registryPath);
    expect(() => preflightRemoteTargets([model], untrusted, allowPolicy())).toThrow(/requires --target-trust/);
    const denied = parseInfrastructurePolicy(JSON.stringify({
      contract: 'woml.infrastructure-policy', contractVersion: 1, id: 'deny-remote', version: '1', environment: 'production',
      defaults: { read: 'deny', 'idempotent-write': 'deny', 'unsafe-write': 'deny' }, rules: [],
    })).bundle;
    const trusted = await loadTargetRegistry(paths.registryPath, paths.trustPath);
    expect(() => preflightRemoteTargets([model], trusted, denied)).toThrow(/denies/);
    if (process.platform !== 'win32') {
      const linked = join(paths.root, 'linked.json');
      await symlink(paths.registryPath, linked);
      await expect(loadTargetRegistry(linked)).rejects.toThrow(/non-symlink/);
    }
  });

  test('exposes bounded, versioned target CLI output without certificate bytes', async () => {
    const paths = await fixture();
    const listed = await invoke(['target', 'list', '--registry', paths.registryPath, '--trust', paths.trustPath, '--json']);
    expect(listed.exitCode).toBe(0);
    const targetList = JSON.parse(listed.stdout);
    expect(targetList.profile).toBe('woml.target-list/v1');
    expect(targetList.targets[0]).toMatchObject({
      id: 'local',
      source: 'built-in',
      kind: 'local',
      eligibleWorkers: 1,
    });
    expect(targetList.targets[1]).toMatchObject({
      id: 'maintenance-runner',
      source: 'operator-registry',
      kind: 'worker-pool',
      minimumWorkerVersion: '1.1.0',
    });
    const checked = await invoke(['target', 'check', 'production-private', '--registry', paths.registryPath, '--trust', paths.trustPath, '--json']);
    expect(checked.exitCode).toBe(0);
    expect(JSON.parse(checked.stdout)).toEqual(expect.objectContaining({ profile: 'woml.target-check/v1', status: 'passed', target: 'production-private' }));
    const inspected = await invoke(['target', 'inspect', 'production-private', '--registry', paths.registryPath, '--trust', paths.trustPath, '--json']);
    expect(inspected.exitCode).toBe(0);
    expect(JSON.parse(inspected.stdout).profile).toBe('woml.target-inspection/v1');
    expect(inspected.stdout).not.toContain('BEGIN CERTIFICATE');
    const local = await invoke(['target', 'check', 'local', '--json']);
    expect(local.exitCode).toBe(0);
    expect(JSON.parse(local.stdout)).toEqual(
      expect.objectContaining({
        profile: 'woml.target-check/v1',
        status: 'passed',
        target: 'local',
        source: 'built-in',
      })
    );
  });

  test('persists explicit target and worker drain intent without abandoning registry state', async () => {
    const paths = await fixture();
    const statePath = join(paths.root, 'remote-drain.json');
    const drained = await invoke([
      'target', 'drain', 'production-private',
      '--registry', paths.registryPath,
      '--trust', paths.trustPath,
      '--state', statePath,
      '--json',
    ]);
    expect(drained.exitCode).toBe(0);
    expect(JSON.parse(drained.stdout)).toEqual(expect.objectContaining({
      profile: 'woml.remote-drain-result/v1',
      status: 'draining',
      kind: 'target',
      id: 'production-private',
    }));
    const inspected = await invoke([
      'target', 'inspect', 'production-private',
      '--registry', paths.registryPath,
      '--trust', paths.trustPath,
      '--state', statePath,
      '--json',
    ]);
    expect(JSON.parse(inspected.stdout).health).toEqual({
      status: 'draining', reason: 'operator-drain', workers: [],
    });
    const state = JSON.parse(await Bun.file(statePath).text());
    expect(state.profile).toBe('woml.remote-drain/v1');
    expect(JSON.stringify(state)).not.toContain('BEGIN CERTIFICATE');
  });

  test('integrates trusted target proof into production check before activation', async () => {
    const paths = await fixture();
    const previous = process.cwd();
    process.chdir(paths.root);
    try {
      const result = await invoke([
        'check',
        paths.workflowPath,
        '--target-registry',
        paths.registryPath,
        '--target-trust',
        paths.trustPath,
        '--policy',
        paths.policyPath,
        '--json',
      ]);
      expect(result.exitCode).toBe(0);
      expect(result.stderr).toBe('');
      expect(JSON.parse(result.stdout)).toEqual(
        expect.objectContaining({
          status: 'passed',
          remoteTargets: expect.objectContaining({
            profile: 'woml.target-preflight/v1',
            status: 'passed',
          }),
        })
      );
    } finally {
      process.chdir(previous);
    }
  });

  test('loads conventional target files only when a compiled workflow needs remote execution', async () => {
    const paths = await fixture();
    const configurationRoot = join(paths.root, '.woml');
    await mkdir(join(configurationRoot, 'certs'), { recursive: true });
    await writeFile(
      join(configurationRoot, 'targets.json'),
      await Bun.file(paths.registryPath).text()
    );
    await writeFile(
      join(configurationRoot, 'target-trust.json'),
      await Bun.file(paths.trustPath).text()
    );
    for (const name of ['production.pem', 'maintenance.pem']) {
      await writeFile(
        join(configurationRoot, 'certs', name),
        TEST_CERTIFICATE
      );
    }
    if (process.platform !== 'win32') {
      await chmod(join(configurationRoot, 'targets.json'), 0o600);
      await chmod(join(configurationRoot, 'target-trust.json'), 0o600);
      await chmod(join(configurationRoot, 'certs', 'production.pem'), 0o644);
      await chmod(join(configurationRoot, 'certs', 'maintenance.pem'), 0o644);
    }
    const previous = process.cwd();
    process.chdir(paths.root);
    try {
      const remote = await invoke([
        'check',
        paths.workflowPath,
        '--policy',
        paths.policyPath,
        '--json',
      ]);
      expect(remote.stderr).toBe('');
      expect(remote.exitCode).toBe(0);
      expect(JSON.parse(remote.stdout).remoteTargets).toMatchObject({
        profile: 'woml.target-preflight/v1',
        status: 'passed',
      });

      await writeFile(join(configurationRoot, 'targets.json'), '{broken');
      const localWorkflow = join(paths.root, 'local.woml');
      await writeFile(
        localWorkflow,
        '<woml><workflow id="local-only"><triggers><manual id="start" /></triggers><steps><step id="work"><script>return { ok: true };</script></step></steps></workflow></woml>'
      );
      const local = await invoke([
        'check',
        localWorkflow,
        '--policy',
        paths.policyPath,
        '--json',
      ]);
      expect(local.exitCode).toBe(0);
      expect(JSON.parse(local.stdout).remoteTargets).toBeUndefined();
    } finally {
      process.chdir(previous);
    }
  });
});

