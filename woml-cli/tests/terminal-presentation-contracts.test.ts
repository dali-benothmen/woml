import { describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';

import Ajv2020 from 'ajv/dist/2020';

import {
  decodeRunPresentationListV1,
  decodeRunPresentationV1,
  RunPresentationDecodeError,
} from '../src/terminal-presentation';

const repositoryRoot = resolve(import.meta.dir, '../..');
const schemaRoot = resolve(repositoryRoot, 'docs/schemas');
const fixtureRoot = resolve(import.meta.dir, 'fixtures/terminal-presentation');

async function validators() {
  const ajv = new Ajv2020({ strict: false, allErrors: true });
  ajv.addFormat('date-time', {
    validate: (value: string) => Number.isFinite(Date.parse(value)),
  });
  const runSchema = await Bun.file(
    resolve(schemaRoot, 'run-presentation.v1.schema.json')
  ).json();
  ajv.addSchema(runSchema);
  const run = ajv.getSchema('https://woml.dev/schemas/run-presentation/v1')!;
  const runList = ajv.compile(
    await Bun.file(resolve(schemaRoot, 'run-presentation-list.v1.schema.json')).json()
  );
  const manual = ajv.compile(
    await Bun.file(resolve(schemaRoot, 'manual-trigger-admission.v1.schema.json')).json()
  );
  return { run, runList, manual };
}

describe('Run Presentation v1 contract', () => {
  test('accepts the reviewed successful and failed run fixtures', async () => {
    const validate = await validators();
    for (const name of [
      'success.v1.json',
      'failure.v1.json',
      'complex-control-flow.v1.json',
    ]) {
      const fixture = await Bun.file(resolve(fixtureRoot, name)).json();
      expect(validate.run(fixture), `${name}: ${JSON.stringify(validate.run.errors)}`).toBe(true);
      expect(() => decodeRunPresentationV1(JSON.stringify(fixture))).not.toThrow();
    }
  });

  test('rejects version drift, unknown fields, invalid times, and synthetic kinds', async () => {
    const validate = await validators();
    const fixture = await Bun.file(resolve(fixtureRoot, 'success.v1.json')).json();
    const cases = [
      { ...structuredClone(fixture), profile: 'woml.run-presentation/v2' },
      { ...structuredClone(fixture), internalSchedulerState: 'running' },
      { ...structuredClone(fixture), admittedAt: 'not-a-date' },
      {
        ...structuredClone(fixture),
        steps: [{ ...structuredClone(fixture.steps[0]), kind: 'compiler_join_barrier' }],
      },
    ];
    for (const value of cases) expect(validate.run(value)).toBe(false);
  });

  test('keeps workflow metadata, business work, and lifecycle work separate', async () => {
    const fixture = await Bun.file(resolve(fixtureRoot, 'success.v1.json')).json();
    expect(fixture.workflow).toMatchObject({
      id: 'order-processing',
      name: 'Order Processing',
      version: '1.4.0',
    });
    expect(fixture.summary).toEqual({
      succeeded: 4,
      failed: 0,
      skipped: 0,
      cancelled: 0,
      total: 4,
    });
    expect(fixture.steps).toHaveLength(4);
    expect(fixture.lifecycle.map((item: { hook: string }) => item.hook)).toEqual([
      'on-success',
      'on-complete',
    ]);
  });

  test('accepts reviewed activation metadata for every supported trigger', async () => {
    const validate = await validators();
    const fixture = await Bun.file(resolve(fixtureRoot, 'success.v1.json')).json();
    const workflow = await Bun.file(resolve(fixtureRoot, 'triggers.v1.json')).json();
    expect(validate.run({
      ...fixture,
      workflow,
      trigger: { id: 'manualStart', type: 'manual' },
    }), JSON.stringify(validate.run.errors)).toBe(true);
    expect(workflow.triggers.map((trigger: { type: string }) => trigger.type)).toEqual([
      'manual', 'webhook', 'event', 'slack', 'schedule', 'interval',
    ]);
  });

  test('strictly decodes one presentation and a workflow-scoped recent list', async () => {
    const validate = await validators();
    const fixture = await Bun.file(resolve(fixtureRoot, 'success.v1.json')).json();
    expect(decodeRunPresentationV1(JSON.stringify(fixture))).toEqual(fixture);
    const list = {
      profile: 'woml.run-presentation-list/v1' as const,
      workflowId: fixture.workflow.id,
      runs: [fixture],
    };
    expect(validate.runList(list), JSON.stringify(validate.runList.errors)).toBe(true);
    expect(decodeRunPresentationListV1(JSON.stringify(list))).toEqual(list);
  });

  test('the decoder fails closed on future profiles, unknown fields, and mismatched lists', async () => {
    const fixture = await Bun.file(resolve(fixtureRoot, 'success.v1.json')).json();
    expect(() => decodeRunPresentationV1(JSON.stringify({
      ...fixture,
      profile: 'woml.run-presentation/v2',
    }))).toThrow(RunPresentationDecodeError);
    expect(() => decodeRunPresentationV1(JSON.stringify({
      ...fixture,
      privateRuntimeField: true,
    }))).toThrow(RunPresentationDecodeError);
    expect(() => decodeRunPresentationListV1(JSON.stringify({
      profile: 'woml.run-presentation-list/v1',
      workflowId: 'another-workflow',
      runs: [fixture],
    }))).toThrow(RunPresentationDecodeError);
  });
});

describe('Manual Trigger Admission v1 contract', () => {
  test('accepts the reviewed request, acceptance, and rejection fixtures', async () => {
    const validate = await validators();
    const fixture = await Bun.file(resolve(fixtureRoot, 'manual-admission.v1.json')).json();
    for (const value of Object.values(fixture)) {
      expect(validate.manual(value), JSON.stringify(validate.manual.errors)).toBe(true);
    }
  });

  test('requires an empty payload and a correlated response identity', async () => {
    const validate = await validators();
    const fixture = await Bun.file(resolve(fixtureRoot, 'manual-admission.v1.json')).json();
    expect(validate.manual({ ...fixture.request, payload: { hiddenDefault: true } })).toBe(false);
    const missingRequestId = structuredClone(fixture.accepted);
    delete missingRequestId.requestId;
    expect(validate.manual(missingRequestId)).toBe(false);
  });
});

describe('Terminal experience semantics', () => {
  test('freezes authority, color, manual, and log-follow invariants', async () => {
    const semantics = await Bun.file(resolve(fixtureRoot, 'semantics.v1.json')).json();
    expect(semantics).toMatchObject({
      presentationAuthority: 'durable_events_and_frozen_definition',
      terminalTextIsAuthority: false,
      colors: {
        autoOnlyForTty: true,
        jsonContainsAnsi: false,
        plainContainsAnsi: false,
      },
      manual: {
        runBeforeInput: false,
        payload: {},
        oneAcceptedSubmissionCreates: 'one_occurrence',
        repeatedSubmissionCreatesDistinctRuns: true,
        oneShotCommand: 'woml test',
        manualOnlyBackgroundAllowed: false,
      },
      logs: {
        workflowHistoryLimit: 10,
        ctrlCStops: 'viewer_only',
        parsesRuntimeLogText: false,
      },
    });
  });
});
