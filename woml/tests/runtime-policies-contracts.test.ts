import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import Ajv2020 from 'ajv/dist/2020';

import {
  buildWomlExecutableDefinitionPackage,
  buildWomlRuntimeDefinitionPackage,
  compileWoml,
  parseWoml,
  WomlCompileError,
  WomlValidationError,
} from '../src';

const repositoryRoot = resolve(import.meta.dir, '../..');
const schemaRoot = resolve(repositoryRoot, 'docs/schemas');
const fixtureRoot = resolve(import.meta.dir, 'fixtures/runtime-policies');

function fixture(name: string): string {
  return readFileSync(resolve(fixtureRoot, name), 'utf8');
}

function workflow(config: string, extra = ''): string {
  return `<woml><workflow id="policy-test">${config}${extra}<triggers><manual id="start" /></triggers><steps><step id="a"><script>return { ok: true };</script></step></steps></workflow></woml>`;
}

function validationError(source: string): WomlValidationError {
  try {
    compileWoml(parseWoml(source, { file: 'policy-invalid.woml' }));
  } catch (error) {
    if (error instanceof WomlValidationError) return error;
    throw error;
  }
  throw new Error('Expected runtime-policy validation to fail.');
}

async function contractValidators() {
  const ajv = new Ajv2020({ strict: false, allErrors: true });
  ajv.addFormat(
    'date-time',
    /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z$/
  );
  const names = [
    ...Array.from(
      { length: 12 },
      (_, index) => `compiled-workflow-model.v${index + 1}.schema.json`
    ),
    ...Array.from(
      { length: 11 },
      (_, index) => `run-event.v${index + 1}.schema.json`
    ),
    ...Array.from(
      { length: 7 },
      (_, index) => `woml-definition-package.v${index + 1}.schema.json`
    ),
    'runtime-policy.v1.schema.json',
    'scheduler-claim.v1.schema.json',
    'runtime-policy-store.v12.schema.json',
    'run-list.v1.schema.json',
    'run-list.v2.schema.json',
    'run-inspection.v2.schema.json',
    'run-inspection.v3.schema.json',
    'runtime-policy-progress.v1.schema.json',
  ];
  for (const name of names) {
    ajv.addSchema(JSON.parse(readFileSync(resolve(schemaRoot, name), 'utf8')));
  }
  return ajv;
}

describe('Frozen runtime-policy contracts', () => {
  test('validates Model v12, Event v11, Store v12, claim, list, inspection, and progress fixtures', async () => {
    const ajv = await contractValidators();
    const model = JSON.parse(fixture('runtime-policy.compiled.v12.json'));
    const contracts = JSON.parse(fixture('contracts.v1.json'));
    const checks = [
      ['https://cronflow.dev/schemas/compiled-workflow-model/v12', model],
      ['https://woml.dev/schemas/runtime-policy/v1', contracts.policy],
      ['https://woml.dev/schemas/scheduler-claim/v1', contracts.claim],
      [
        'https://woml.dev/schemas/runtime-policy-store/v12',
        contracts.queuedRecord,
      ],
      ['https://woml.dev/schemas/run-list/v2', contracts.list],
      ['https://woml.dev/schemas/run-inspection/v3', contracts.inspection],
      [
        'https://woml.dev/schemas/runtime-policy-progress/v1',
        contracts.progress,
      ],
    ] as const;
    for (const [id, value] of checks) {
      const validate = ajv.getSchema(id)!;
      expect(validate(value), `${id}: ${JSON.stringify(validate.errors)}`).toBe(
        true
      );
    }

    const validateEvent = ajv.getSchema(
      'https://cronflow.dev/schemas/run-event/v11'
    )!;
    for (const event of JSON.parse(fixture('events.v11.json'))) {
      expect(validateEvent(event), JSON.stringify(validateEvent.errors)).toBe(
        true
      );
    }
  });

  test('freezes admission before one first start and timeout before failed outcome', () => {
    const events = JSON.parse(fixture('events.v11.json')) as readonly {
      type: string;
      sequence: number;
      data: Record<string, unknown>;
    }[];
    expect(events.map(event => event.sequence)).toEqual([1, 2, 3, 4, 5]);
    expect(events[0].type).toBe('run_admitted');
    expect(
      events.filter(event => event.type === 'run_execution_started')
    ).toHaveLength(1);
    expect(
      events.findIndex(event => event.type === 'run_timeout_reached')
    ).toBeLessThan(
      events.findIndex(event => event.type === 'run_outcome_decided')
    );
    expect(events.at(-1)?.type).toBe('run_finalized');
  });

  test('freezes wait/resume, cancellation, crash, conflict, and queue-overflow boundaries', () => {
    const sequences = JSON.parse(
      fixture('event-sequences.v11.json')
    ) as Readonly<Record<string, readonly string[]>>;
    for (const [name, sequence] of Object.entries(sequences)) {
      if (name === 'policyConflict') {
        expect(sequence).toEqual([]);
        continue;
      }
      expect(sequence[0], name).toBe('run_admitted');
      expect(sequence.at(-1), name).toBe('run_finalized');
      expect(
        sequence.filter(type => type === 'run_execution_started').length,
        name
      ).toBeLessThanOrEqual(1);
    }
    expect(sequences.queuedCancellation).not.toContain('run_execution_started');
    expect(sequences.approvalWaitResume).toContain('approval_resolved');
    expect(sequences.ownerCrash).toContain('step_attempt_failed');

    const overflow = JSON.parse(fixture('overflow-transports.v1.json'));
    expect(overflow).toMatchObject({
      code: 'WOML_POLICY_QUEUE_FULL',
      retryable: true,
      runCreated: false,
      transports: {
        webhook: { httpStatus: 503, retryAfter: true },
        slack: { acknowledge: false },
        schedule: { advanceCursor: false },
        event: { subscriberStatus: 'retryable_failure' },
        workflowCall: { operationFailure: 'retryable' },
      },
    });
  });

  test('keeps runtime policy outside the DAG and private scheduler ownership out of public inspection', async () => {
    const ajv = await contractValidators();
    const model = JSON.parse(fixture('runtime-policy.compiled.v12.json'));
    model.graph.runtimePolicy = model.runtimePolicy;
    const validateModel = ajv.getSchema(
      'https://cronflow.dev/schemas/compiled-workflow-model/v12'
    )!;
    expect(validateModel(model)).toBe(false);

    const inspection = JSON.parse(fixture('contracts.v1.json')).inspection;
    inspection.policy.ownerId = 'private-owner';
    const validateInspection = ajv.getSchema(
      'https://woml.dev/schemas/run-inspection/v3'
    )!;
    expect(validateInspection(inspection)).toBe(false);
  });
});

describe('Runtime-policy frontend', () => {
  test('deep-equals the reviewed Model v12 fixture', () => {
    const compiled = compileWoml(
      parseWoml(fixture('runtime-policy.woml'), {
        file: 'runtime-policy.woml',
      })
    );
    expect(compiled).toEqual(
      JSON.parse(fixture('runtime-policy.compiled.v12.json'))
    );
    expect(compiled.schemaVersion).toBe(12);
    if (compiled.schemaVersion === 12) {
      expect(compiled.runtimePolicy.rateLimit?.algorithm).toBe(
        'rolling_window'
      );
      expect(compiled.graph).not.toHaveProperty('runtimePolicy');
    }
  });

  test('normalizes every policy attribute and accepts fractional source durations that resolve to whole milliseconds', () => {
    const compiled = compileWoml(
      parseWoml(
        workflow(
          '<config concurrency="1" timeout="0.5s" rate-limit="2/0.5m" queue="agents.inbound" />'
        )
      )
    );
    expect(compiled.schemaVersion).toBe(12);
    if (compiled.schemaVersion !== 12) throw new Error('expected Model v12');
    expect(compiled.runtimePolicy).toEqual({
      profileVersion: 1,
      concurrency: 1,
      timeoutMs: 500,
      rateLimit: {
        count: 2,
        windowMs: 30_000,
        algorithm: 'rolling_window',
      },
      queue: {
        name: 'agents.inbound',
        discipline: 'work_conserving_fifo',
      },
    });
  });

  test('reports precise config diagnostics', () => {
    const cases = [
      [workflow('<config />'), 'WOML_CONFIG_EMPTY'],
      [
        workflow('<config concurrency="0" />'),
        'WOML_CONFIG_CONCURRENCY_INVALID',
      ],
      [workflow('<config timeout="10" />'), 'WOML_CONFIG_TIMEOUT_INVALID'],
      [
        workflow('<config rate-limit="100" />'),
        'WOML_CONFIG_RATE_LIMIT_INVALID',
      ],
      [workflow('<config queue="Orders" />'), 'WOML_CONFIG_QUEUE_INVALID'],
      [workflow('<config priority="high" />'), 'WOML_CONFIG_ATTRIBUTE_UNKNOWN'],
      [
        workflow('<config concurrency="1"><script>return;</script></config>'),
        'WOML_CONFIG_CHILD_NOT_ALLOWED',
      ],
      [
        workflow('<config concurrency="1" /><config timeout="1m" />'),
        'WOML_CONFIG_DUPLICATE',
      ],
    ] as const;
    for (const [source, code] of cases) {
      expect(validationError(source).diagnostic.code, source).toBe(code);
    }

    const reordered = compileWoml(
      parseWoml(
        '<woml><workflow id="policy-test"><triggers><manual id="start" /></triggers><config concurrency="1" /><steps><step id="a"><script>return { ok: true };</script></step></steps></workflow></woml>'
      )
    );
    expect(reordered.schemaVersion).toBe(12);
    if (reordered.schemaVersion === 12) {
      expect(reordered.runtimePolicy?.concurrency).toBe(1);
    }
  });

  test('composes with call-only and lifecycle workflows without changing the DAG interface', () => {
    const callOnly = compileWoml(
      parseWoml(
        '<woml><workflow id="call-policy"><config concurrency="1" /><steps><step id="a"><script>return context.payload;</script></step></steps></workflow></woml>'
      )
    );
    expect(callOnly.schemaVersion).toBe(12);
    expect(callOnly.triggers).toEqual([]);

    const lifecycle = compileWoml(
      parseWoml(
        workflow(
          '<config timeout="1m" />',
          '<lifecycle><on-success><script>console.log("done");</script></on-success></lifecycle>'
        )
      )
    );
    expect(lifecycle.schemaVersion).toBe(12);
    if (lifecycle.schemaVersion !== 12) throw new Error('expected Model v12');
    expect(lifecycle.lifecycle?.hooks).toHaveLength(1);
  });

  test('preserves legacy compilation when config is absent', () => {
    const compiled = compileWoml(
      parseWoml(
        '<woml><workflow id="legacy"><triggers><manual id="start" /></triggers><steps><step id="a"><script>return { ok: true };</script></step></steps></workflow></woml>'
      )
    );
    expect(compiled.schemaVersion).toBe(1);
    expect(compiled).not.toHaveProperty('runtimePolicy');
  });

  test('emits Definition Package v7 and keeps Model v12 module runtime packaging staged', async () => {
    const file = resolve(fixtureRoot, 'runtime-policy-module.woml');
    const document = parseWoml(readFileSync(file, 'utf8'), { file });
    const definitionPackage = await buildWomlExecutableDefinitionPackage(
      document,
      { sourcePath: file, projectRoot: fixtureRoot }
    );
    expect(definitionPackage.schemaVersion).toBe(7);
    expect(definitionPackage.workflow.model.schemaVersion).toBe(12);

    const ajv = await contractValidators();
    const validate = ajv.getSchema(
      'https://woml.dev/schemas/woml-definition-package.v7.schema.json'
    )!;
    expect(validate(definitionPackage), JSON.stringify(validate.errors)).toBe(
      true
    );

    try {
      await buildWomlRuntimeDefinitionPackage(document, {
        sourcePath: file,
        projectRoot: fixtureRoot,
      });
      throw new Error('Expected runtime-policy execution to remain gated.');
    } catch (error) {
      expect(error).toBeInstanceOf(WomlCompileError);
      expect((error as WomlCompileError).diagnostic.code).toBe(
        'WOML_RUNTIME_POLICY_RUNTIME_UNAVAILABLE'
      );
    }
  });
});
