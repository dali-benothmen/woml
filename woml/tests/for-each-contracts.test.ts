import { describe, expect, test } from 'bun:test';
import { readdirSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import Ajv2020 from 'ajv/dist/2020';

const repositoryRoot = resolve(import.meta.dir, '../..');
const schemaRoot = resolve(repositoryRoot, 'docs/schemas');
const fixtureRoot = resolve(import.meta.dir, 'fixtures/for-each');

function json(path: string): any {
  return JSON.parse(readFileSync(path, 'utf8'));
}

function fixture(name: string): any {
  return json(resolve(fixtureRoot, name));
}

function validators(): Ajv2020 {
  const ajv = new Ajv2020({ strict: false, allErrors: true });
  ajv.addFormat(
    'date-time',
    /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z$/
  );
  for (const name of readdirSync(schemaRoot)
    .filter(name => name.endsWith('.schema.json'))
    .sort()) {
    ajv.addSchema(json(resolve(schemaRoot, name)));
  }
  return ajv;
}

function expectValid(
  ajv: Ajv2020,
  schemaId: string,
  value: unknown
): void {
  const validate = ajv.getSchema(schemaId);
  if (!validate) throw new Error(`Missing schema ${schemaId}.`);
  expect(validate(value), JSON.stringify(validate.errors, null, 2)).toBe(true);
}

describe('Frozen for-each contracts', () => {
  test('validates the reviewed Model v16, Event v15, and Script Host v9 artifacts', () => {
    const ajv = validators();
    expectValid(
      ajv,
      'https://woml.dev/schemas/compiled-workflow-model/v16',
      fixture('model.v16.reviewed.json')
    );
    for (const event of fixture('events.v15.reviewed.json')) {
      expectValid(
        ajv,
        'https://woml.dev/schemas/run-event/v15',
        event
      );
    }
    for (const message of fixture('script-host.v9.reviewed.json')) {
      expectValid(
        ajv,
        'https://woml.dev/schemas/script-host-protocol/v9',
        message
      );
    }
  });

  test('pins limits, identity, result ordering, visibility, and recovery policy', () => {
    expect(fixture('semantics.v1.json')).toEqual({
      versions: { model: 16, event: 15, scriptHost: 9 },
      limits: {
        defaultConcurrency: 1,
        maximumConcurrency: 64,
        maximumItems: 10000,
      },
      execution: {
        identity: ['runId', 'forEachId', 'index'],
        resultOrder: 'input-order',
        failurePolicy: 'fail-fast-then-settle-owned-work',
        recoveryAuthority: 'event-fold',
        interruptedAttempts: 'fail-closed',
        emptyInput: 'succeed-immediately',
      },
      visibility: {
        bodyOutputs: 'iteration-local',
        outerOutput: 'for-each-id-only',
      },
      unsupported: [
        'nested-for-each',
        'fork-in-for-each',
        'approval-in-for-each',
        'streaming-iterables',
        'dynamic-concurrency',
        'continue-on-error',
      ],
    });

    const model = fixture('model.v16.reviewed.json');
    expect(model.graph.forEach[0].body.contextVisibility).toEqual([
      { nodeId: 'normalize', stepIds: [] },
    ]);
    expect(
      model.graph.contextVisibility.find(
        (entry: any) => entry.nodeId === 'summary'
      ).stepIds
    ).toEqual(['load', 'organize']);

    const iterationResults = fixture('events.v15.reviewed.json')
      .filter((event: any) => event.type === 'for_each_iteration_succeeded')
      .sort((left: any, right: any) => left.iteration.index - right.iteration.index)
      .map((event: any) => event.data.result.value);
    expect(iterationResults).toEqual(['first', 'second']);
  });

  test('rejects invalid descriptor limits and iteration-reference escapes', () => {
    const ajv = validators();
    const validate = ajv.getSchema(
      'https://woml.dev/schemas/compiled-workflow-model/v16'
    )!;
    const model = fixture('model.v16.reviewed.json');

    expect(
      validate({
        ...model,
        graph: {
          ...model.graph,
          forEach: [{ ...model.graph.forEach[0], concurrency: 65 }],
        },
      })
    ).toBe(false);
    expect(
      validate({
        ...model,
        graph: {
          ...model.graph,
          forEach: [
            {
              ...model.graph.forEach[0],
              items: {
                kind: 'contextReference',
                path: ['item', 'children'],
              },
            },
          ],
        },
      })
    ).toBe(false);
  });

  test('requires bounded iteration scope and keeps raw inputs out of loop-control events', () => {
    const ajv = validators();
    const validateEvent = ajv.getSchema(
      'https://woml.dev/schemas/run-event/v15'
    )!;
    const event = fixture('events.v15.reviewed.json')[1];
    const { iteration: _iteration, ...withoutIteration } = event;
    expect(validateEvent(withoutIteration)).toBe(false);
    expect(validateEvent({ ...event, data: { ...event.data, item: 'raw' } })).toBe(
      false
    );

    const serialized = JSON.stringify(fixture('events.v15.reviewed.json'));
    expect(serialized).not.toContain('secret');
    expect(serialized).not.toContain('rawContext');
  });

  test('requires item and iteration to cross the Script Host v9 boundary together', () => {
    const ajv = validators();
    const validate = ajv.getSchema(
      'https://woml.dev/schemas/script-host-protocol/v9'
    )!;
    const root = fixture('script-host.v9.reviewed.json')[0];
    const loop = fixture('script-host.v9.reviewed.json')[1];

    expect(validate(root), JSON.stringify(validate.errors)).toBe(true);
    expect(validate(loop), JSON.stringify(validate.errors)).toBe(true);
    expect(
      validate({
        ...loop,
        context: { ...loop.context, iteration: undefined },
      })
    ).toBe(false);
    expect(
      validate({
        ...loop,
        context: {
          ...loop.context,
          iteration: { index: 10000, total: 2 },
        },
      })
    ).toBe(false);
  });
});
