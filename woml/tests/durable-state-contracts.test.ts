import { describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';

import Ajv2020 from 'ajv/dist/2020';

const repositoryRoot = resolve(import.meta.dir, '../..');
const schemaRoot = resolve(repositoryRoot, 'docs/schemas');
const fixtureRoot = resolve(import.meta.dir, 'fixtures/durable-state');

async function validator() {
  const ajv = new Ajv2020({ strict: false, allErrors: true });
  ajv.addFormat('date-time', {
    validate: (value: string) => Number.isFinite(Date.parse(value)),
  });
  for (const name of [
    'capability-call.v1.schema.json',
    'durable-state.v1.schema.json',
    'durable-state-mutation-identity.v1.schema.json',
    'state-operation-metadata.v1.schema.json',
    'durable-state-store.v13.schema.json',
  ]) {
    ajv.addSchema(await Bun.file(resolve(schemaRoot, name)).json());
  }
  return ajv;
}

describe('DS0 frozen Durable User State contracts', () => {
  test('validates every reviewed request, result, identity, metadata, and Store v13 record', async () => {
    const ajv = await validator();
    const contracts = await Bun.file(resolve(fixtureRoot, 'contracts.v1.json')).json();
    const validateState = ajv.getSchema('https://woml.dev/schemas/durable-state/v1')!;
    for (const value of [
      ...Object.values(contracts.requests),
      ...Object.values(contracts.results),
    ]) {
      expect(validateState(value), JSON.stringify(validateState.errors)).toBe(true);
    }

    for (const [id, file] of [
      ['https://woml.dev/schemas/durable-state-mutation-identity/v1', 'identity.v1.json'],
      ['https://woml.dev/schemas/state-operation-metadata/v1', 'metadata.v1.json'],
      ['https://woml.dev/schemas/durable-state-store/v13', 'store.v13.json'],
    ] as const) {
      const validate = ajv.getSchema(id)!;
      const fixture = await Bun.file(resolve(fixtureRoot, file)).json();
      for (const value of Array.isArray(fixture) ? fixture : [fixture]) {
        expect(validate(value), `${id}: ${JSON.stringify(validate.errors)}`).toBe(true);
      }
    }
  });

  test('proves Capability Call v1 carries State v1 without widening the generic protocol', async () => {
    const ajv = await validator();
    const call = await Bun.file(resolve(fixtureRoot, 'capability-call.v1.json')).json();
    const validateCall = ajv.getSchema('https://cronflow.dev/schemas/capability-call/v1')!;
    expect(validateCall(call), JSON.stringify(validateCall.errors)).toBe(true);
    expect(call).toMatchObject({
      capability: 'state',
      operation: 'set',
      inputContractVersion: 1,
      resultContractVersion: 1,
      identity: { mode: 'named' },
    });
  });

  test('freezes fail-closed conflict, quota, interruption, and staged-runtime failures', async () => {
    const failures = (await Bun.file(resolve(fixtureRoot, 'failures.v1.json')).json()) as
      readonly Record<string, unknown>[];
    expect(failures.map(failure => failure.code)).toEqual([
      'WOML_STATE_CONFLICT',
      'WOML_STATE_OPERATION_IDENTITY_CONFLICT',
      'WOML_STATE_QUOTA_EXCEEDED',
      'WOML_STATE_INTERRUPTED',
      'WOML_STATE_RUNTIME_UNAVAILABLE',
    ]);
    expect(failures.find(failure => failure.code === 'WOML_STATE_INTERRUPTED')).toMatchObject({
      retryable: false,
      ambiguous: true,
    });
  });

  test('redacted metadata cannot contain raw keys, values, scopes, runs, or operation identities', async () => {
    const ajv = await validator();
    const validate = ajv.getSchema('https://woml.dev/schemas/state-operation-metadata/v1')!;
    for (const forbidden of ['key', 'value', 'scope', 'workflowId', 'runId', 'operationKey']) {
      expect(
        validate({
          profile: 'woml.state-operation-metadata/v1',
          operation: 'set',
          keyDigest: `sha256:${'a'.repeat(64)}`,
          inputDigest: `sha256:${'b'.repeat(64)}`,
          outcome: 'succeeded',
          durationMs: 1,
          [forbidden]: 'sensitive',
        })
      ).toBe(false);
    }
  });

  test('keeps cache semantics out of State v1', async () => {
    const ajv = await validator();
    const validate = ajv.getSchema('https://woml.dev/schemas/durable-state/v1')!;
    expect(
      validate({
        contract: 'woml.state',
        contractVersion: 1,
        kind: 'request',
        operation: 'set',
        input: { key: 'customer', value: 1, ttlMs: 60_000 },
      })
    ).toBe(false);
  });
});
