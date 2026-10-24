import { afterEach, describe, expect, test } from 'bun:test';
import {
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  symlink,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';

import type { AnySchema } from 'ajv';
import Ajv2020 from 'ajv/dist/2020';

import {
  assertWomlProjectOptionCompatibility,
  parseWomlProject,
  resolveWomlProject,
  WOML_PROJECT_MAX_BYTES,
  WomlProjectError,
} from '../src/project';

const repositoryRoot = resolve(import.meta.dir, '../..');
const fixtureRoot = resolve(
  repositoryRoot,
  'woml/tests/fixtures/combined-developer-experience'
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
  await Promise.all(
    temporary.splice(0).map(path => rm(path, { recursive: true, force: true }))
  );
});

async function tempProject(prefix = 'woml-project-v1-'): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), prefix));
  temporary.push(root);
  await mkdir(join(root, 'workflows'), { recursive: true });
  return root;
}

function workflow(id: string, lineEnding = '\n'): string {
  return [
    '<woml>',
    `  <workflow id="${id}">`,
    '    <triggers><manual id="start" /></triggers>',
    '    <steps><step id="done"><script>return true;</script></step></steps>',
    '  </workflow>',
    '</woml>',
    '',
  ].join(lineEnding);
}

async function writeJson(path: string, value: unknown): Promise<void> {
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, `${JSON.stringify(value, null, 2)}\n`);
  if (process.platform !== 'win32') await chmod(path, 0o600);
}

async function writeManifest(root: string, value: unknown): Promise<string> {
  const path = join(root, 'woml.project.json');
  await writeJson(path, value);
  return path;
}

function minimalManifest(workflows: readonly string[] = ['./workflows']) {
  return {
    profile: 'woml.project/v1',
    version: 1,
    name: 'portable-project',
    workflows,
  };
}

async function expectProjectError(
  operation: Promise<unknown> | (() => unknown),
  code: string
): Promise<void> {
  try {
    if (typeof operation === 'function') operation();
    else await operation;
    throw new Error(`Expected ${code}`);
  } catch (error) {
    expect(error).toBeInstanceOf(WomlProjectError);
    expect(error).toMatchObject({ code });
    expect(String(error)).toContain('Corrective action:');
  }
}

describe('Combined Developer Experience v1 foundation', () => {
  test('validates the reviewed Project and Project Check fixtures', async () => {
    const ajv = new Ajv2020({ allErrors: true, strict: false });
    for (const [schemaName, fixtureName] of [
      ['woml-project.v1.schema.json', 'project.v1.reviewed.json'],
      ['project-check.v1.schema.json', 'project-check.v1.reviewed.json'],
    ] as const) {
      const schema = JSON.parse(
        await readFile(resolve(repositoryRoot, 'docs/schemas', schemaName), 'utf8')
      ) as AnySchema;
      const fixture = JSON.parse(
        await readFile(resolve(fixtureRoot, fixtureName), 'utf8')
      ) as unknown;
      const validate = ajv.compile(schema);
      expect(
        validate(fixture),
        `${fixtureName}: ${ajv.errorsText(validate.errors)}`
      ).toBe(true);
    }

    const budget = JSON.parse(
      await readFile(resolve(fixtureRoot, 'budgets.v1.json'), 'utf8')
    ) as Record<string, unknown>;
    expect(budget.profile).toBe(
      'woml.combined-developer-experience-budgets/v1'
    );
  });

  test('rejects future versions, unknown secret fields, duplicate keys, and unsafe references', () => {
    expect(() =>
      parseWomlProject(
        JSON.stringify({ ...minimalManifest(), version: 2 })
      )
    ).toThrow(expect.objectContaining({ code: 'WOML_PROJECT_UNSUPPORTED_VERSION' }));
    expect(() =>
      parseWomlProject(
        JSON.stringify({ ...minimalManifest(), token: 'must-not-enter-projects' })
      )
    ).toThrow(expect.objectContaining({ code: 'WOML_PROJECT_INVALID' }));
    expect(() =>
      parseWomlProject(
        '{"profile":"woml.project/v1","version":1,"name":"one","name":"two","workflows":["./workflows"]}'
      )
    ).toThrow(expect.objectContaining({ code: 'WOML_PROJECT_INVALID' }));
    expect(() =>
      parseWomlProject(' '.repeat(WOML_PROJECT_MAX_BYTES + 1))
    ).toThrow(expect.objectContaining({ code: 'WOML_PROJECT_INVALID' }));
    for (const reference of [
      '/etc/workflows',
      'C:/workflows',
      '../workflows',
      '.\\workflows',
    ]) {
      expect(() =>
        parseWomlProject(JSON.stringify(minimalManifest([reference])))
      ).toThrow(expect.objectContaining({ code: 'WOML_PROJECT_INVALID' }));
    }
    expect(() =>
      parseWomlProject(
        JSON.stringify({
          ...minimalManifest(),
          authority: { infrastructurePolicy: './policy.json' },
        })
      )
    ).toThrow(expect.objectContaining({ code: 'WOML_PROJECT_INVALID' }));
    expect(() =>
      parseWomlProject(
        JSON.stringify({
          ...minimalManifest(),
          remoteExecution: { targetRegistry: './targets.json' },
        })
      )
    ).toThrow(expect.objectContaining({ code: 'WOML_PROJECT_INVALID' }));
  });

  test('resolves portable workflow identity independently of root, key order, and line endings', async () => {
    const firstRoot = await tempProject('woml-project-first-');
    const secondRoot = await tempProject('woml-project-second-');
    await writeFile(join(firstRoot, 'workflows', 'b.woml'), workflow('beta'));
    await writeFile(join(firstRoot, 'workflows', 'a.woml'), workflow('alpha'));
    await writeFile(
      join(secondRoot, 'workflows', 'b.woml'),
      workflow('beta', '\r\n')
    );
    await writeFile(
      join(secondRoot, 'workflows', 'a.woml'),
      workflow('alpha', '\r\n')
    );
    const firstPath = await writeManifest(firstRoot, minimalManifest());
    const secondPath = await writeManifest(secondRoot, {
      workflows: ['./workflows'],
      name: 'portable-project',
      version: 1,
      profile: 'woml.project/v1',
    });

    const first = await resolveWomlProject(firstPath);
    const second = await resolveWomlProject(secondPath);
    expect(first.digest).toBe(second.digest);
    expect(first.digest).toMatch(/^sha256:[a-f0-9]{64}$/);
    expect(first.workflows.map(item => item.projectPath)).toEqual([
      'workflows/a.woml',
      'workflows/b.woml',
    ]);
    expect(first.workflows.flatMap(item => item.workflowIds)).toEqual([
      'alpha',
      'beta',
    ]);
    expect(first.manifestPath.startsWith(firstRoot)).toBe(true);
    expect(first.digest).not.toContain(firstRoot);
  });

  test('rejects missing, escaped, duplicated, oversized, and conflicting workflow inputs', async () => {
    const root = await tempProject();
    await writeFile(join(root, 'workflows', 'one.woml'), workflow('duplicate'));
    const manifestPath = await writeManifest(
      root,
      minimalManifest(['./workflows', './workflows/one.woml'])
    );
    await expectProjectError(
      resolveWomlProject(manifestPath),
      'WOML_PROJECT_DUPLICATE_WORKFLOW'
    );

    await writeFile(join(root, 'workflows', 'two.woml'), workflow('duplicate'));
    await writeManifest(root, minimalManifest());
    await expectProjectError(
      resolveWomlProject(manifestPath),
      'WOML_PROJECT_DUPLICATE_WORKFLOW'
    );

    await rm(join(root, 'workflows', 'two.woml'));
    const outside = await tempProject('woml-project-outside-');
    await writeFile(join(outside, 'escaped.woml'), workflow('escaped'));
    await symlink(
      join(outside, 'escaped.woml'),
      join(root, 'workflows', 'escaped.woml')
    );
    await expectProjectError(
      resolveWomlProject(manifestPath),
      'WOML_PROJECT_PATH_ESCAPE'
    );

    await rm(join(root, 'workflows', 'escaped.woml'));
    await writeManifest(root, minimalManifest(['./missing.woml']));
    await expectProjectError(
      resolveWomlProject(manifestPath),
      'WOML_PROJECT_NOT_FOUND'
    );

    const manifest = parseWomlProject(JSON.stringify(minimalManifest()));
    expect(() =>
      assertWomlProjectOptionCompatibility(manifest, { runtime: true })
    ).not.toThrow();
    const withRuntime = parseWomlProject(
      JSON.stringify({ ...minimalManifest(), runtime: './runtime.json' })
    );
    await expectProjectError(
      () => assertWomlProjectOptionCompatibility(withRuntime, { runtime: true }),
      'WOML_PROJECT_OPTION_CONFLICT'
    );
  });

  test('loads every referenced configuration through its existing authority loader', async () => {
    const root = await tempProject();
    await writeFile(join(root, 'workflows', 'deploy.woml'), workflow('deploy'));
    await mkdir(join(root, 'policies'), { recursive: true });
    await mkdir(join(root, 'trust'), { recursive: true });
    await mkdir(join(root, 'targets', 'certificates'), { recursive: true });

    await writeJson(join(root, 'runtime.json'), {
      schemaVersion: 2,
      deploymentName: 'project-test',
      statePath: '.woml/state.sqlite',
      logging: { directory: '.woml/logs' },
    });
    await writeJson(join(root, 'policies', 'process.json'), {
      contract: 'woml.process-policy',
      contractVersion: 1,
      enabled: true,
      modes: { foreground: true, background: true },
      projectRoot: '..',
      executables: [{ alias: 'bun', path: process.execPath }],
      workingDirectories: ['.'],
      environment: { inherit: [], allow: [] },
      limits: {
        maxTimeoutMs: 60_000,
        maxOutputBytes: 262_144,
        maxStreamBytes: 1_048_576,
        maxConcurrent: 2,
      },
    });
    await writeJson(join(root, 'policies', 'artifacts.json'), {
      contract: 'woml.artifact-policy',
      contractVersion: 1,
      enabled: true,
      modes: { foreground: true, background: true },
      projectRoot: '..',
      readRoots: ['.'],
      materialization: {
        directory: '.woml/materialized',
        cleanupOnStart: true,
        maxBytesPerAttempt: 1_048_576,
      },
      limits: {
        maxArtifactBytes: 1_048_576,
        maxRunBytes: 2_097_152,
        maxStoreBytes: 4_194_304,
        maxConcurrentTransfers: 2,
        transferTimeoutMs: 60_000,
      },
      retention: {
        defaultMs: 86_400_000,
        minimumMs: 3_600_000,
        maximumMs: 604_800_000,
      },
      secretScanning: { knownSecretAction: 'reject', minimumSecretBytes: 8 },
      backend: {
        contract: 'woml.artifact-backend',
        contractVersion: 1,
        kind: 'local',
        directory: '.woml/artifacts',
      },
    });
    await writeJson(join(root, 'policies', 'infrastructure.json'), {
      contract: 'woml.infrastructure-policy',
      contractVersion: 1,
      id: 'project-test',
      version: '1',
      environment: 'test',
      defaults: {
        read: 'deny',
        'idempotent-write': 'deny',
        'unsafe-write': 'deny',
      },
      rules: [],
    });
    await writeJson(join(root, 'policies', 'emergency-deny.json'), {
      contract: 'woml.infrastructure-policy',
      contractVersion: 1,
      id: 'project-test-emergency',
      version: '1',
      environment: 'test',
      defaults: {
        read: 'allow',
        'idempotent-write': 'allow',
        'unsafe-write': 'allow',
      },
      rules: [
        {
          id: 'deny-emergency-writes',
          match: { effects: ['unsafe-write'] },
          decision: 'deny',
        },
      ],
    });
    await writeJson(join(root, 'trust', 'policy.json'), {});
    await writeJson(join(root, 'reviewers.json'), {
      profile: 'woml.reviewer-directory/v1',
      version: 'test-1',
      principals: [],
    });
    await writeJson(join(root, 'targets', 'registry.json'), {
      profile: 'woml.target-registry/v1',
      version: 1,
      targets: [
        {
          id: 'production-private',
          kind: 'worker-pool',
          allowedWorkerIdentities: ['worker-01'],
          requiredCapabilities: ['runtime.script'],
          minimumWorkerVersion: '1.0.0',
          maximumInFlight: 1,
        },
      ],
    });
    await writeJson(join(root, 'targets', 'trust.json'), {
      profile: 'woml.target-trust/v1',
      version: 1,
      identities: [
        {
          workerIdentity: 'worker-01',
          certificatePath: 'certificates/worker-01.pem',
        },
      ],
    });
    await writeFile(
      join(root, 'targets', 'certificates', 'worker-01.pem'),
      TEST_CERTIFICATE
    );
    if (process.platform !== 'win32') {
      await chmod(
        join(root, 'targets', 'certificates', 'worker-01.pem'),
        0o644
      );
    }
    const manifestPath = await writeManifest(root, {
      ...minimalManifest(),
      runtime: './runtime.json',
      authority: {
        processPolicy: './policies/process.json',
        artifactPolicy: './policies/artifacts.json',
        infrastructurePolicy: './policies/infrastructure.json',
        policyTrust: './trust/policy.json',
        emergencyDeny: './policies/emergency-deny.json',
        reviewerDirectory: './reviewers.json',
      },
      remoteExecution: {
        targetRegistry: './targets/registry.json',
        targetTrust: './targets/trust.json',
      },
    });

    const project = await resolveWomlProject(manifestPath, {
      mode: 'background',
      environment: {},
    });
    expect(project.runtime?.deploymentName).toBe('project-test');
    expect(project.authorities.process?.process.mode).toBe('background');
    expect(project.authorities.artifacts?.artifacts.policy.backend.kind).toBe(
      'local'
    );
    expect(
      project.authorities.infrastructure?.infrastructurePolicy.policy.id
    ).toBe('project-test');
    expect(project.authorities.infrastructure?.emergencyDeny?.policy.id).toBe(
      'project-test-emergency'
    );
    expect(project.authorities.reviewers?.reviewers.directory.version).toBe(
      'test-1'
    );
    expect(project.authorities.targets?.trust?.identities['worker-01']).toBeDefined();
    expect(Object.keys(project.references)).toEqual([
      'artifactPolicy',
      'emergencyDeny',
      'infrastructurePolicy',
      'policyTrust',
      'processPolicy',
      'reviewerDirectory',
      'runtime',
      'targetRegistry',
      'targetTrust',
    ]);
  });
});
