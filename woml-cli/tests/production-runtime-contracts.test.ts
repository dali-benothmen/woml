import { describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';

import Ajv2020 from 'ajv/dist/2020';

const repositoryRoot = resolve(import.meta.dir, '../..');
const schemaRoot = resolve(repositoryRoot, 'docs/schemas');
const fixtureRoot = resolve(import.meta.dir, 'fixtures/production-runtime');

const schemas = [
  'runtime-configuration.v1.schema.json',
  'production-preflight.v1.schema.json',
  'deployment-activation.v1.schema.json',
  'background-runtime-control.v1.schema.json',
  'runtime-instance.v1.schema.json',
  'production-runtime-store.v14.schema.json',
  'runtime-descriptor.v1.schema.json',
  'runtime-admin-http.v1.schema.json',
  'runtime-operations-snapshot.v1.schema.json',
  'runtime-operations-stream.v1.schema.json',
  'runtime-log-record.v1.schema.json',
  'runtime-metrics.v1.schema.json',
  'runtime-health.v1.schema.json',
  'backup-manifest.v1.schema.json',
  'retention.v1.schema.json',
] as const;

async function validators() {
  const ajv = new Ajv2020({ strict: false, allErrors: true });
  ajv.addFormat('date-time', { validate: (value: string) => Number.isFinite(Date.parse(value)) });
  for (const name of schemas) {
    ajv.addSchema(await Bun.file(resolve(schemaRoot, name)).json());
  }
  return ajv;
}

describe('Frozen Production Runtime contracts', () => {
  test('validates every reviewed production contract and Store v14 record', async () => {
    const ajv = await validators();
    const config = await Bun.file(resolve(fixtureRoot, 'runtime-config.v1.json')).json();
    const contracts = await Bun.file(resolve(fixtureRoot, 'contracts.v1.json')).json();
    const values = [
      ['https://woml.dev/schemas/runtime-configuration/v1', config],
      ['https://woml.dev/schemas/production-preflight/v1', contracts.preflight],
      ['https://woml.dev/schemas/deployment-activation/v1', contracts.activation],
      ['https://woml.dev/schemas/background-runtime-control/v1', contracts.backgroundStarted],
      ['https://woml.dev/schemas/background-runtime-control/v1', contracts.backgroundStop],
      ['https://woml.dev/schemas/runtime-instance/v1', contracts.runtimeInstance],
      ['https://woml.dev/schemas/runtime-descriptor/v1', contracts.descriptor],
      ['https://woml.dev/schemas/runtime-admin-http/v1', contracts.adminRequest],
      ['https://woml.dev/schemas/runtime-admin-http/v1', contracts.adminResponse],
      ['https://woml.dev/schemas/production-runtime-store/v14', contracts.store],
      ['https://woml.dev/schemas/runtime-operations-snapshot/v1', contracts.snapshot],
      ['https://woml.dev/schemas/runtime-operations-stream/v1', contracts.stream],
      ['https://woml.dev/schemas/runtime-log-record/v1', contracts.log],
      ['https://woml.dev/schemas/runtime-metrics/v1', contracts.metric],
      ['https://woml.dev/schemas/runtime-health/v1', contracts.liveness],
      ['https://woml.dev/schemas/runtime-health/v1', contracts.readiness],
      ['https://woml.dev/schemas/runtime-health/v1', contracts.healthDetail],
      ['https://woml.dev/schemas/backup-manifest/v1', contracts.backup],
      ['https://woml.dev/schemas/retention/v1', contracts.retentionPlan],
      ['https://woml.dev/schemas/retention/v1', contracts.retentionResult],
    ] as const;
    for (const [id, value] of values) {
      const validate = ajv.getSchema(id)!;
      expect(validate(value), `${id}: ${JSON.stringify(validate.errors)}`).toBe(true);
    }
  });

  test('freezes startup, shutdown, background, readiness, and retention invariants', async () => {
    const semantics = await Bun.file(resolve(fixtureRoot, 'semantics.v1.json')).json();
    expect(semantics.startup.at(-1)).toBe('readiness_opened');
    expect(semantics.shutdown.slice(0, 2)).toEqual(['readiness_closed', 'ingress_closed']);
    expect(semantics.shutdown.indexOf('durable_truth_settled')).toBeLessThan(
      semantics.shutdown.indexOf('ownership_released')
    );
    expect(semantics.background).toMatchObject({
      stopTargets: 'runtime_instance_id',
      pidAloneIsAuthority: false,
      survivesTerminalClose: true,
      survivesMachineRestart: false,
    });
    expect(semantics.retentionNeverOwns).toContain('services_state');
  });

  test('rejects sensitive or unbounded observability and retention fields', async () => {
    const ajv = await validators();
    const contracts = await Bun.file(resolve(fixtureRoot, 'contracts.v1.json')).json();

    const snapshot = structuredClone(contracts.snapshot);
    snapshot.runs[0].payload = { card: 'secret' };
    expect(ajv.getSchema('https://woml.dev/schemas/runtime-operations-snapshot/v1')!(snapshot)).toBe(false);

    const metric = structuredClone(contracts.metric);
    metric.labels.run_id = 'run_01';
    expect(ajv.getSchema('https://woml.dev/schemas/runtime-metrics/v1')!(metric)).toBe(false);

    const retention = structuredClone(contracts.retentionResult);
    retention.stateEntriesDeleted = 1;
    expect(ajv.getSchema('https://woml.dev/schemas/retention/v1')!(retention)).toBe(false);

    const config = await Bun.file(resolve(fixtureRoot, 'runtime-config.v1.json')).json();
    config.secret = 'forbidden';
    expect(ajv.getSchema('https://woml.dev/schemas/runtime-configuration/v1')!(config)).toBe(false);
  });

  test('keeps the administration descriptor loopback-only and instance-scoped', async () => {
    const ajv = await validators();
    const contracts = await Bun.file(resolve(fixtureRoot, 'contracts.v1.json')).json();
    const descriptor = { ...contracts.descriptor, adminUrl: 'http://0.0.0.0:3001' };
    expect(ajv.getSchema('https://woml.dev/schemas/runtime-descriptor/v1')!(descriptor)).toBe(false);
    expect(contracts.adminRequest.operation).toBe('stop');
    expect(contracts.backgroundStop.runtimeInstanceId).toBe(contracts.descriptor.runtimeInstanceId);
  });
});
