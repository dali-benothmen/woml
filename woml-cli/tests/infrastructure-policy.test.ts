import { mkdir, mkdtemp, readFile, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { relative, resolve } from 'node:path';

import { describe, expect, test } from 'bun:test';
import type { AnySchema } from 'ajv';
import Ajv2020 from 'ajv/dist/2020';
import addFormats from 'ajv-formats';

import {
  InfrastructurePolicyError,
  canonicalizeInfrastructurePolicy,
  parseInfrastructurePolicy,
  prepareInfrastructurePolicy,
  resolveInfrastructurePolicy,
  validateEmergencyDenyOverlay,
} from '../src/infrastructure-policy';
import {
  analyzeInfrastructurePolicySource,
  infrastructurePolicyDiagnostics,
} from '../src/infrastructure-policy-analysis';

const repositoryRoot = resolve(import.meta.dir, '../..');
const fixtureRoot = resolve(
  repositoryRoot,
  'woml/tests/fixtures/infrastructure-policies'
);

async function json(path: string): Promise<unknown> {
  return JSON.parse(await readFile(path, 'utf8')) as unknown;
}

describe('Infrastructure Policy v1 foundation', () => {
  test('resolves project policy names while rejecting extensions and symlink escapes', async () => {
    const root = await mkdtemp(resolve(tmpdir(), 'woml-policy-resolution-'));
    const outside = await mkdtemp(resolve(tmpdir(), 'woml-policy-outside-'));
    await mkdir(resolve(root, '.woml/policies'), { recursive: true });
    const source = await readFile(
      resolve(fixtureRoot, 'production.unsigned.v1.json'),
      'utf8'
    );
    await writeFile(resolve(root, '.woml/policies/production.json'), source);
    await mkdir(resolve(root, 'policies with spaces'), { recursive: true });
    const unicodePolicy = resolve(
      root,
      'policies with spaces',
      'π production.json'
    );
    await writeFile(unicodePolicy, source);
    await writeFile(resolve(outside, 'escaped.json'), source);
    expect(
      (await resolveInfrastructurePolicy('production', { projectRoot: root }))
        .prepared.bundle.id
    ).toBe('production');
    await symlink(
      resolve(outside, 'escaped.json'),
      resolve(root, '.woml/policies/escaped.json')
    );
    await expect(
      resolveInfrastructurePolicy('escaped', { projectRoot: root })
    ).rejects.toMatchObject({
      code: 'WOML_POLICY_INVALID',
    });
    await expect(
      resolveInfrastructurePolicy('production.yaml', { projectRoot: root })
    ).rejects.toMatchObject({
      code: 'WOML_POLICY_INVALID',
    });
    expect(
      (
        await resolveInfrastructurePolicy(relative(root, unicodePolicy), {
          projectRoot: root,
        })
      ).prepared.bundle.id
    ).toBe('production');
  });

  test('keeps static explanations conservative and rejects an allowing emergency overlay', async () => {
    const prepared = parseInfrastructurePolicy(
      await readFile(
        resolve(fixtureRoot, 'production.unsigned.v1.json'),
        'utf8'
      )
    );
    const analysis = analyzeInfrastructurePolicySource(
      prepared.bundle,
      'deploy',
      '<script>await services.process.run({ executable: tool, args })</script>'
    );
    expect(analysis).toHaveLength(1);
    expect(analysis[0]?.status).not.toBe('allowed');
    expect(infrastructurePolicyDiagnostics(prepared)).toEqual([]);

    const overlay = prepareInfrastructurePolicy({
      ...prepared.bundle,
      defaults: {
        read: 'allow',
        'idempotent-write': 'allow',
        'unsafe-write': 'allow',
      },
      rules: [
        {
          id: 'bad-allow',
          match: { capabilities: ['process'] },
          decision: 'allow',
        },
      ],
    });
    expect(() => validateEmergencyDenyOverlay(overlay)).toThrow(
      InfrastructurePolicyError
    );
  });

  test('validates the frozen unsigned and signed fixtures against JSON Schema', async () => {
    const schema = (await json(
      resolve(
        repositoryRoot,
        'docs/schemas/infrastructure-policy.v1.schema.json'
      )
    )) as AnySchema;
    const ajv = new Ajv2020({ allErrors: true, strict: false });
    addFormats(ajv);
    const validate = ajv.compile(schema);
    for (const name of [
      'production.unsigned.v1.json',
      'production.signed.v1.json',
    ]) {
      const fixture = await json(resolve(fixtureRoot, name));
      expect(
        validate(fixture),
        `${name}: ${ajv.errorsText(validate.errors)}`
      ).toBe(true);
    }
  });

  test('freezes canonical identity independently of object-key order', async () => {
    const source = await readFile(
      resolve(fixtureRoot, 'production.unsigned.v1.json'),
      'utf8'
    );
    const identity = (await json(resolve(fixtureRoot, 'identity.v1.json'))) as {
      digest: string;
    };
    const prepared = parseInfrastructurePolicy(source);
    expect(prepared.digest).toBe(identity.digest);
    expect(prepared.signatureStatus).toBe('unsigned');

    const reordered = {
      rules: prepared.bundle.rules,
      environment: prepared.bundle.environment,
      defaults: prepared.bundle.defaults,
      version: prepared.bundle.version,
      id: prepared.bundle.id,
      contractVersion: prepared.bundle.contractVersion,
      contract: prepared.bundle.contract,
    };
    const second = prepareInfrastructurePolicy(reordered);
    expect(second.digest).toBe(identity.digest);
    expect(canonicalizeInfrastructurePolicy(second.bundle)).toBe(
      Buffer.from(prepared.canonicalPayload).toString('utf8')
    );
  });

  test('verifies the frozen Ed25519 signature using only operator trust', async () => {
    const source = await readFile(
      resolve(fixtureRoot, 'production.signed.v1.json'),
      'utf8'
    );
    const identity = (await json(resolve(fixtureRoot, 'identity.v1.json'))) as {
      digest: string;
      signingKeyId: string;
      publicKey: string;
    };
    const prepared = parseInfrastructurePolicy(source, {
      keys: { [identity.signingKeyId]: identity.publicKey },
      requireSignature: true,
    });
    expect(prepared.digest).toBe(identity.digest);
    expect(prepared.signatureStatus).toBe('verified');
    expect(prepared.signingKeyId).toBe(identity.signingKeyId);
  });

  test('fails closed for missing, unknown, revoked, or altered signatures', async () => {
    const unsigned = await readFile(
      resolve(fixtureRoot, 'production.unsigned.v1.json'),
      'utf8'
    );
    const signed = await readFile(
      resolve(fixtureRoot, 'production.signed.v1.json'),
      'utf8'
    );
    const identity = (await json(resolve(fixtureRoot, 'identity.v1.json'))) as {
      signingKeyId: string;
      publicKey: string;
    };
    expect(() =>
      parseInfrastructurePolicy(unsigned, { requireSignature: true })
    ).toThrow(InfrastructurePolicyError);
    for (const trust of [
      {},
      {
        keys: { [identity.signingKeyId]: identity.publicKey },
        revokedKeyIds: [identity.signingKeyId],
      },
    ]) {
      try {
        parseInfrastructurePolicy(signed, trust);
        throw new Error('expected signature rejection');
      } catch (error) {
        expect(error).toMatchObject({ code: 'WOML_POLICY_SIGNATURE_INVALID' });
      }
    }
    const altered = signed.replace('2026-10-22.1', '2026-10-22.2');
    expect(() =>
      parseInfrastructurePolicy(altered, {
        keys: { [identity.signingKeyId]: identity.publicKey },
      })
    ).toThrow(InfrastructurePolicyError);
  });

  test('rejects duplicate keys, unknown fields, future versions, and ambiguous rules', async () => {
    const source = await readFile(
      resolve(fixtureRoot, 'production.unsigned.v1.json'),
      'utf8'
    );
    const duplicate = source.replace(
      '"contractVersion": 1,',
      '"contractVersion": 1, "contractVersion": 1,'
    );
    expect(() => parseInfrastructurePolicy(duplicate)).toThrow(
      InfrastructurePolicyError
    );

    const base = JSON.parse(source) as Record<string, unknown>;
    expect(() =>
      prepareInfrastructurePolicy({ ...base, surprise: true })
    ).toThrow(InfrastructurePolicyError);
    expect(() =>
      prepareInfrastructurePolicy({ ...base, contractVersion: 2 })
    ).toThrow(InfrastructurePolicyError);

    const rules = structuredClone(base.rules) as Record<string, unknown>[];
    rules[0] = { ...rules[0], match: {} };
    expect(() => prepareInfrastructurePolicy({ ...base, rules })).toThrow(
      InfrastructurePolicyError
    );

    const emptySet = structuredClone(base.rules) as Record<string, unknown>[];
    emptySet[0] = {
      ...emptySet[0],
      match: { capabilities: [], effects: ['unsafe-write'] },
    };
    expect(() =>
      prepareInfrastructurePolicy({ ...base, rules: emptySet })
    ).toThrow(InfrastructurePolicyError);

    const denyWithRequirements = structuredClone(base.rules) as Record<
      string,
      unknown
    >[];
    denyWithRequirements[0] = {
      ...denyWithRequirements[0],
      decision: 'deny',
    };
    expect(() =>
      prepareInfrastructurePolicy({ ...base, rules: denyWithRequirements })
    ).toThrow(InfrastructurePolicyError);

    expect(() => parseInfrastructurePolicy(`\u00a0${source}`)).toThrow(
      InfrastructurePolicyError
    );
    expect(() =>
      parseInfrastructurePolicy(
        source.replace(
          'Allow only a reviewed Terraform apply while the deployment lock is valid.',
          String.fromCharCode(0xd800)
        )
      )
    ).toThrow(InfrastructurePolicyError);
    expect(() =>
      parseInfrastructurePolicy(`${'['.repeat(18)}null${']'.repeat(18)}`)
    ).toThrow(InfrastructurePolicyError);
    expect(() =>
      parseInfrastructurePolicy(' '.repeat(1024 * 1024 + 1))
    ).toThrow(InfrastructurePolicyError);
  });

  test('keeps the IP0 coverage and threat model explicit and machine-reviewable', async () => {
    const coverage = (await json(resolve(fixtureRoot, 'coverage.v1.json'))) as {
      paths: { path: string; posture: string }[];
    };
    const threatModel = (await json(
      resolve(fixtureRoot, 'threat-model.v1.json')
    )) as {
      trustedAuthorities: string[];
      untrustedSources: string[];
      invariants: string[];
    };
    expect(new Set(coverage.paths.map(path => path.path)).size).toBe(
      coverage.paths.length
    );
    expect(coverage.paths.find(path => path.path === 'fetch')?.posture).toBe(
      'protect-or-block'
    );
    expect(threatModel.trustedAuthorities).toContain('rust-runtime');
    expect(threatModel.untrustedSources).toContain('ai-output');
    expect(threatModel.invariants.length).toBeGreaterThanOrEqual(10);
  });

  test('analyzes StateService setIfAbsent using the canonical policy operation', () => {
    const policy = prepareInfrastructurePolicy({
      contract: 'woml.infrastructure-policy', contractVersion: 1,
      id: 'state-analysis', version: '1', environment: 'production',
      defaults: { read: 'deny', 'idempotent-write': 'deny', 'unsafe-write': 'deny' },
      rules: [
        { id: 'state-reads', match: { capabilities: ['state'], operations: ['get'] }, decision: 'allow' },
        { id: 'stop-initialization', match: { capabilities: ['state'], operations: ['set_if_absent'] }, decision: 'deny' },
      ],
    });
    const results = analyzeInfrastructurePolicySource(policy.bundle, 'state-workflow',
      'services.state.setIfAbsent("private-key", {}, { name: "initialize" }); services.state.get("private-key");');
    expect(results.map(result => [result.operation, result.status])).toEqual([
      ['get', 'allowed'], ['set_if_absent', 'denied'],
    ]);
    expect(JSON.stringify(results)).not.toContain('private-key');
  });

  test('classifies definite allow, deny, requirements, and dynamic operations conservatively', () => {
    const base = {
      contract: 'woml.infrastructure-policy',
      contractVersion: 1,
      id: 'classification',
      version: '1',
      environment: 'production',
      defaults: {
        read: 'deny',
        'idempotent-write': 'deny',
        'unsafe-write': 'deny',
      },
    } as const;
    const source = '<script>services.http.request({});</script>';
    const status = (rules: readonly Record<string, unknown>[]) =>
      analyzeInfrastructurePolicySource(
        prepareInfrastructurePolicy({ ...base, rules }).bundle,
        'deploy',
        source
      )[0]?.status;

    expect(
      status([
        {
          id: 'allow-http',
          match: { capabilities: ['http'], operations: ['request'] },
          decision: 'allow',
        },
      ])
    ).toBe('allowed');
    expect(
      status([
        {
          id: 'deny-http',
          match: { capabilities: ['http'], operations: ['request'] },
          decision: 'deny',
        },
      ])
    ).toBe('denied');
    expect(
      status([
        {
          id: 'review-http',
          match: { capabilities: ['http'], operations: ['request'] },
          decision: 'allow',
          requires: {
            approval: {
              minimum: 1,
              roles: [],
              bindReviewDigest: false,
              maxAgeMs: 60_000,
            },
          },
        },
      ])
    ).toBe('requirements_missing');
    expect(
      status([
        {
          id: 'targeted-http',
          match: {
            capabilities: ['http'],
            operations: ['request'],
            targets: ['api.example.com'],
          },
          decision: 'allow',
        },
      ])
    ).toBe('unknown_until_runtime');
  });
});
