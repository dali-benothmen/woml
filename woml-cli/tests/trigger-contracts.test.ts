import { describe, expect, test } from 'bun:test';
import { readdirSync } from 'node:fs';
import { join, resolve } from 'node:path';

import Ajv2020, { type ValidateFunction } from 'ajv/dist/2020';
import addFormats from 'ajv-formats';
import { compileWoml, parseWoml } from 'woml';

type JsonObject = Record<string, unknown>;

const projectRoot = resolve(import.meta.dir, '../..');
const schemaDirectory = resolve(projectRoot, 'docs/schemas');
const fixtureDirectory = resolve(projectRoot, 'woml/tests/fixtures');
const contractFixtureDirectory = join(fixtureDirectory, 'trigger-contracts');

async function readJson(path: string): Promise<unknown> {
  return JSON.parse(await Bun.file(path).text());
}

function canonicalizeJson(value: unknown): string {
  if (value === null || typeof value !== 'object') {
    const encoded = JSON.stringify(value);
    if (encoded === undefined) throw new Error('Value is not JSON.');
    return encoded;
  }
  if (Array.isArray(value)) {
    return `[${value.map(canonicalizeJson).join(',')}]`;
  }
  const object = value as JsonObject;
  return `{${Object.keys(object)
    .sort()
    .map(key => `${JSON.stringify(key)}:${canonicalizeJson(object[key])}`)
    .join(',')}}`;
}

function sha256Canonical(value: unknown): string {
  return `sha256:${new Bun.CryptoHasher('sha256')
    .update(canonicalizeJson(value))
    .digest('hex')}`;
}

function sha256Text(value: string): string {
  return `sha256:${new Bun.CryptoHasher('sha256')
    .update(value)
    .digest('hex')}`;
}

function errors(validate: ValidateFunction): string {
  return JSON.stringify(validate.errors, null, 2);
}

async function validators(): Promise<{
  readonly model: ValidateFunction;
  readonly event: ValidateFunction;
  readonly occurrence: ValidateFunction;
  readonly ingress: ValidateFunction;
  readonly webhook: ValidateFunction;
  readonly slack: ValidateFunction;
  readonly progress: ValidateFunction;
}> {
  const ajv = new Ajv2020({ allErrors: true, strict: false });
  addFormats(ajv);
  const names = [
    'attempt-failure.v1.schema.json',
    'attempt-failure.v2.schema.json',
    'compiled-workflow-model.v1.schema.json',
    'compiled-workflow-model.v2.schema.json',
    'compiled-workflow-model.v3.schema.json',
    'compiled-workflow-model.v4.schema.json',
    'compiled-workflow-model.v5.schema.json',
    'compiled-workflow-model.v6.schema.json',
    'compiled-workflow-model.v7.schema.json',
    'run-event.v1.schema.json',
    'run-event.v2.schema.json',
    'run-event.v3.schema.json',
    'run-event.v4.schema.json',
    'run-event.v5.schema.json',
    'run-event.v6.schema.json',
    'run-event.v7.schema.json',
    'trigger-occurrence.v1.schema.json',
    'trigger-ingress.v1.schema.json',
    'webhook-http.v1.schema.json',
    'slack-trigger-protocol.v1.schema.json',
    'trigger-progress.v1.schema.json',
  ];
  for (const name of names) {
    ajv.addSchema((await readJson(join(schemaDirectory, name))) as JsonObject);
  }
  return {
    model: ajv.getSchema(
      'https://cronflow.dev/schemas/compiled-workflow-model/v7'
    )!,
    event: ajv.getSchema('https://cronflow.dev/schemas/run-event/v7')!,
    occurrence: ajv.getSchema(
      'https://cronflow.dev/schemas/trigger-occurrence/v1'
    )!,
    ingress: ajv.getSchema(
      'https://cronflow.dev/schemas/trigger-ingress/v1'
    )!,
    webhook: ajv.getSchema(
      'https://cronflow.dev/schemas/webhook-http/v1'
    )!,
    slack: ajv.getSchema(
      'https://cronflow.dev/schemas/slack-trigger-protocol/v1'
    )!,
    progress: ajv.getSchema(
      'https://cronflow.dev/schemas/trigger-progress/v1'
    )!,
  };
}

describe('T0 production trigger contracts', () => {
  test('all reviewed trigger models validate and pin the six handler shapes', async () => {
    const { model } = await validators();
    const fixtureNames = [
      'triggers-webhook.compiled.v7.json',
      'triggers-slack.compiled.v7.json',
      'triggers-schedule.compiled.v7.json',
      'triggers-interval.compiled.v7.json',
      'triggers-event.compiled.v7.json',
    ];
    const handlers = new Set<string>();
    for (const name of fixtureNames) {
      const fixture = (await readJson(join(fixtureDirectory, name))) as JsonObject;
      expect(model(fixture), `${name}: ${errors(model)}`).toBe(true);
      for (const trigger of fixture.triggers as JsonObject[]) {
        handlers.add(String(trigger.handler));
      }
    }
    expect([...handlers].sort()).toEqual([
      'trigger.event',
      'trigger.interval',
      'trigger.manual',
      'trigger.schedule',
      'trigger.slack',
      'trigger.webhook',
    ]);
  });

  test('T1 compiles the reviewed webhook source exactly to Model v7', async () => {
    const source = await Bun.file(
      join(fixtureDirectory, 'triggers-webhook.woml')
    ).text();
    const expected = await readJson(
      join(fixtureDirectory, 'triggers-webhook.compiled.v7.json')
    );
    const compiled = compileWoml(
      parseWoml(source, { file: 'triggers-webhook.woml' })
    );
    expect(compiled).toEqual(expected as typeof compiled);
    expect(compiled.schemaVersion).toBe(7);
  });

  test('Model v7 rejects resolved secrets and confused trigger configs', async () => {
    const { model } = await validators();
    const webhook = (await readJson(
      join(fixtureDirectory, 'triggers-webhook.compiled.v7.json')
    )) as JsonObject;
    const resolved = structuredClone(webhook);
    const webhookTrigger = (resolved.triggers as JsonObject[])[1];
    const webhookFields = ((webhookTrigger.config as JsonObject)
      .fields as JsonObject);
    const authFields = ((webhookFields.authentication as JsonObject)
      .fields as JsonObject);
    authFields.secret = { kind: 'literal', value: 'actual-secret-token' };
    expect(model(resolved)).toBe(false);

    const wrongMethod = structuredClone(webhook);
    const wrongFields = ((((wrongMethod.triggers as JsonObject[])[1]
      .config as JsonObject).fields) as JsonObject);
    wrongFields.method = { kind: 'literal', value: 'GET' };
    expect(model(wrongMethod)).toBe(false);

    const slack = (await readJson(
      join(fixtureDirectory, 'triggers-slack.compiled.v7.json')
    )) as JsonObject;
    const slackFields = ((((slack.triggers as JsonObject[])[0]
      .config as JsonObject).fields) as JsonObject);
    slackFields.botToken = { kind: 'literal', value: 'xoxb-secret' };
    expect(model(slack)).toBe(false);
  });

  test('Event v7 binds the direct trigger payload to one occurrence', async () => {
    const { event } = await validators();
    const history = (await readJson(
      join(fixtureDirectory, 'run-events/webhook-trigger.events.v7.json')
    )) as JsonObject[];
    for (const [index, item] of history.entries()) {
      expect(event(item), errors(event)).toBe(true);
      expect(item.eventSchemaVersion).toBe(7);
      expect(item.runId).toBe('run_webhook_001');
      expect(item.sequence).toBe(index + 1);
    }
    expect(history[0].type).toBe('run_started');
    expect(history.at(-1)?.type).toBe('run_succeeded');
    const start = history[0].data as JsonObject;
    expect(start.triggerId).toBe('newOrder');
    expect(start.triggerHandler).toBe('trigger.webhook');
    expect(start.triggerOccurrenceId).toBe('occ_webhook_001');
    expect(start.trigger).toEqual({ orderId: 'order-42' });

    const missingOccurrence = structuredClone(history[0]);
    delete (missingOccurrence.data as JsonObject).triggerOccurrenceId;
    expect(event(missingOccurrence)).toBe(false);
  });

  test('occurrence, ingress, HTTP, Slack, and progress fixtures conform', async () => {
    const all = await validators();
    const mapping: Readonly<Record<string, ValidateFunction>> = {
      occurrence: all.occurrence,
      ingress: all.ingress,
      webhook: all.webhook,
      slack: all.slack,
      progress: all.progress,
    };
    for (const name of readdirSync(contractFixtureDirectory).sort()) {
      const prefix = name.split('-')[0];
      const validate = mapping[prefix];
      expect(validate, `No validator mapped for ${name}`).toBeDefined();
      const fixture = await readJson(join(contractFixtureDirectory, name));
      expect(validate(fixture), `${name}: ${errors(validate)}`).toBe(true);
    }
  });

  test('accepted, duplicate, conflict, restart, and corrupt-history outcomes stay distinct', async () => {
    const accepted = (await readJson(
      join(contractFixtureDirectory, 'ingress-accepted.v1.json')
    )) as JsonObject;
    const duplicate = (await readJson(
      join(contractFixtureDirectory, 'ingress-duplicate.v1.json')
    )) as JsonObject;
    const afterRestart = (await readJson(
      join(contractFixtureDirectory, 'ingress-restart-duplicate.v1.json')
    )) as JsonObject;
    const conflict = (await readJson(
      join(contractFixtureDirectory, 'ingress-conflict.v1.json')
    )) as JsonObject;
    const corrupt = (await readJson(
      join(contractFixtureDirectory, 'ingress-corrupt-history.v1.json')
    )) as JsonObject;

    expect(accepted.duplicate).toBe(false);
    expect(duplicate.duplicate).toBe(true);
    expect(afterRestart.duplicate).toBe(true);
    expect(afterRestart.occurrenceId).toBe(duplicate.occurrenceId);
    expect(afterRestart.runId).toBe(duplicate.runId);
    expect((conflict.failure as JsonObject).code).toBe(
      'WOML_TRIGGER_IDEMPOTENCY_CONFLICT'
    );
    expect((corrupt.failure as JsonObject).code).toBe(
      'WOML_TRIGGER_HISTORY_INVALID'
    );
  });

  test('the reviewed definition hash and durable record never store raw source keys', async () => {
    const model = await readJson(
      join(fixtureDirectory, 'triggers-webhook.compiled.v7.json')
    );
    const ingress = (await readJson(
      join(contractFixtureDirectory, 'ingress-admit.v1.json')
    )) as JsonObject;
    const occurrenceText = await Bun.file(
      join(contractFixtureDirectory, 'occurrence-webhook.v1.json')
    ).text();
    const occurrence = JSON.parse(occurrenceText) as JsonObject;
    expect(ingress.definitionHash).toBe(sha256Canonical(model));
    expect(occurrence.definitionHash).toBe(ingress.definitionHash);
    expect(occurrence.sourceIdentityHash).toBe(
      sha256Text(String(ingress.sourceIdentity))
    );
    expect(occurrence.payloadHash).toBe(sha256Canonical(ingress.payload));
    expect(occurrenceText).not.toContain(String(ingress.sourceIdentity));
    expect(occurrenceText).not.toContain('ORDER_WEBHOOK_TOKEN');

    const slackModel = await readJson(
      join(fixtureDirectory, 'triggers-slack.compiled.v7.json')
    );
    const slackEvent = (await readJson(
      join(contractFixtureDirectory, 'slack-event.v1.json')
    )) as JsonObject;
    expect(slackEvent.definitionHash).toBe(sha256Canonical(slackModel));
  });
});
