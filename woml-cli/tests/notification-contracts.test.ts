import { describe, expect, test } from 'bun:test';
import { createHash } from 'node:crypto';
import { readdir } from 'node:fs/promises';
import { join, resolve } from 'node:path';

import Ajv2020, { type ValidateFunction } from 'ajv/dist/2020';
import addFormats from 'ajv-formats';

type JsonObject = Record<string, unknown>;

const projectRoot = resolve(import.meta.dir, '../..');
const schemasDirectory = resolve(projectRoot, 'docs/schemas');

async function readJson(path: string): Promise<JsonObject> {
  return (await Bun.file(path).json()) as JsonObject;
}

async function validators(): Promise<{
  readonly model: ValidateFunction;
  readonly event: ValidateFunction;
  readonly provider: ValidateFunction;
  readonly diagnostics: ValidateFunction;
}> {
  const names = [
    'attempt-failure.v1.schema.json',
    'attempt-failure.v2.schema.json',
    'compiled-workflow-model.v1.schema.json',
    'compiled-workflow-model.v2.schema.json',
    'compiled-workflow-model.v3.schema.json',
    'compiled-workflow-model.v4.schema.json',
    'compiled-workflow-model.v5.schema.json',
    'run-event.v1.schema.json',
    'run-event.v2.schema.json',
    'run-event.v3.schema.json',
    'run-event.v4.schema.json',
    'run-event.v5.schema.json',
    'notification-provider-host.v1.schema.json',
    'notification-journey-diagnostics.v1.schema.json',
  ];
  const schemas = await Promise.all(
    names.map(name => readJson(join(schemasDirectory, name)))
  );
  const ajv = new Ajv2020({ strict: false, allErrors: true });
  addFormats(ajv);
  for (const schema of schemas) ajv.addSchema(schema);
  return {
    model: ajv.getSchema(
      'https://cronflow.dev/schemas/compiled-workflow-model/v5'
    )!,
    event: ajv.getSchema('https://cronflow.dev/schemas/run-event/v5')!,
    provider: ajv.getSchema(
      'https://cronflow.dev/schemas/notification-provider-host/v1'
    )!,
    diagnostics: ajv.getSchema(
      'https://cronflow.dev/schemas/notification-journey-diagnostics/v1'
    )!,
  };
}

function canonicalize(value: unknown): string {
  if (value === null || typeof value !== 'object') {
    const encoded = JSON.stringify(value);
    if (encoded === undefined) throw new Error('Value is not JSON.');
    return encoded;
  }
  if (Array.isArray(value)) {
    return `[${value.map(canonicalize).join(',')}]`;
  }
  const object = value as JsonObject;
  return `{${Object.keys(object)
    .sort()
    .map(key => `${JSON.stringify(key)}:${canonicalize(object[key])}`)
    .join(',')}}`;
}

function definitionHash(value: unknown): string {
  return `sha256:${createHash('sha256').update(canonicalize(value)).digest('hex')}`;
}

function errors(validator: ValidateFunction): string {
  return JSON.stringify(validator.errors, null, 2);
}

describe('Slack notification N0 contracts', () => {
  test('pins the secret-safe N6.1 journey diagnostic envelope', async () => {
    const { diagnostics } = await validators();
    const value = {
      version: 1,
      deliveryFailures: [
        {
          deliveryId: 'releaseApproval:notify:0:channel:1',
          provider: 'slack',
          destination: '#engineering',
          attempt: 1,
          final: true,
          failure: {
            kind: 'provider_auth_failed',
            code: 'WOML_SLACK_PERMISSION_DENIED',
            message:
              'Slack operation conversations.list needs additional app permissions. Missing scopes: channels:read.',
            retryable: false,
          },
        },
      ],
    };
    expect(diagnostics(value), errors(diagnostics)).toBe(true);

    for (const [provider, marker, destination] of [
      ['telegram', 'chat', '-1001234567890'],
      ['discord', 'channel', '200000000000000001'],
      ['whatsapp', 'recipient', '15551234567'],
      ['custom', 'channel', 'local-approval'],
    ] as const) {
      const mixed = structuredClone(value);
      const failure = mixed.deliveryFailures[0]!;
      failure.provider = provider;
      failure.deliveryId = `releaseApproval:notify:1:${marker}:0`;
      failure.destination = destination;
      expect(diagnostics(mixed), `${provider}: ${errors(diagnostics)}`).toBe(
        true
      );
    }

    const leaked = structuredClone(value) as JsonObject;
    (leaked.deliveryFailures as JsonObject[])[0]!.decisionCapability =
      'ncap_forbidden';
    expect(diagnostics(leaked)).toBe(false);
    expect(JSON.stringify(value)).not.toMatch(/xox[baprs]-|ncap_|authorization/i);
  });

  test('pins the reviewed model-v5 delivery expansion and canonical hash', async () => {
    const { model } = await validators();
    const compiled = await readJson(
      resolve(
        projectRoot,
        'woml/tests/fixtures/approval-slack.compiled.v5.json'
      )
    );

    expect(model(compiled), errors(model)).toBe(true);
    expect(definitionHash(compiled)).toBe(
      'sha256:a02f094f7200f0e7e33bef7de2aba9b52638ac24adb9f017fd292764fbcb6988'
    );

    const nodes = (compiled.graph as JsonObject).nodes as JsonObject[];
    const wait = nodes.find(node => node.id === 'releaseApproval')!;
    const fields = (wait.inputs as JsonObject).fields as JsonObject;
    const deliveries = (fields.notifications as JsonObject)
      .items as JsonObject[];
    expect(deliveries).toHaveLength(2);
    expect(
      deliveries.map(delivery => {
        const deliveryFields = delivery.fields as JsonObject;
        return {
          id: (deliveryFields.deliveryId as JsonObject).value,
          destination: (deliveryFields.destination as JsonObject).value,
        };
      })
    ).toEqual([
      {
        id: 'releaseApproval:notify:0:channel:0',
        destination: '#approvals',
      },
      {
        id: 'releaseApproval:notify:0:channel:1',
        destination: '#engineering',
      },
    ]);

    const serialized = JSON.stringify(compiled);
    expect(serialized).toContain('SLACK_BOT_TOKEN');
    expect(serialized).toContain('SLACK_APP_TOKEN');
    expect(serialized).not.toMatch(/xox[baprs]-|secretValue|botTokenValue/i);

    const singleChannel = structuredClone(compiled);
    const singleFields = (
      ((singleChannel.graph as JsonObject).nodes as JsonObject[])[0]
        .inputs as JsonObject
    ).fields as JsonObject;
    (singleFields.notifications as JsonObject).items = [
      ((singleFields.notifications as JsonObject).items as JsonObject[])[0],
    ];
    expect(model(singleChannel), errors(model)).toBe(true);

    const multiWorkspace = structuredClone(compiled);
    const multiFields = (
      ((multiWorkspace.graph as JsonObject).nodes as JsonObject[])[0]
        .inputs as JsonObject
    ).fields as JsonObject;
    const multiDeliveries = (multiFields.notifications as JsonObject)
      .items as JsonObject[];
    const secondWorkspace = multiDeliveries[1].fields as JsonObject;
    (secondWorkspace.deliveryId as JsonObject).value =
      'releaseApproval:notify:1:channel:0';
    (secondWorkspace.destination as JsonObject).value = 'C87654321';
    const secondCredentials = (secondWorkspace.credentials as JsonObject)
      .fields as JsonObject;
    secondCredentials.botToken = {
      kind: 'secretReference',
      name: 'CUSTOMER_SLACK_BOT_TOKEN',
    };
    secondCredentials.appToken = {
      kind: 'secretReference',
      name: 'CUSTOMER_SLACK_APP_TOKEN',
    };
    expect(model(multiWorkspace), errors(model)).toBe(true);
    expect(
      multiDeliveries.map(
        delivery =>
          ((delivery.fields as JsonObject).deliveryId as JsonObject).value
      )
    ).toEqual([
      'releaseApproval:notify:0:channel:0',
      'releaseApproval:notify:1:channel:0',
    ]);

    const literalCredential = structuredClone(compiled);
    const literalFields = (
      ((literalCredential.graph as JsonObject).nodes as JsonObject[])[0]
        .inputs as JsonObject
    ).fields as JsonObject;
    const firstDelivery = (
      (literalFields.notifications as JsonObject).items as JsonObject[]
    )[0].fields as JsonObject;
    const credentials = (firstDelivery.credentials as JsonObject)
      .fields as JsonObject;
    credentials.botToken = { kind: 'literal', value: 'forbidden' };
    expect(model(literalCredential)).toBe(false);
  });

  test('pins approved, partial-success, and explicit all-failed event histories', async () => {
    const { event } = await validators();
    const directory = resolve(projectRoot, 'woml/tests/fixtures/run-events');
    const names = [
      'approval-slack-partial-delivery.events.v5.json',
      'approval-slack-all-failed.events.v5.json',
      'approval-slack-approved.events.v5.json',
    ];

    for (const name of names) {
      const history = (await Bun.file(
        join(directory, name)
      ).json()) as JsonObject[];
      history.forEach((item, index) => {
        expect(event(item), `${name}[${index}]: ${errors(event)}`).toBe(true);
        expect(item.sequence).toBe(index + 1);
      });
      expect(JSON.stringify(history)).not.toMatch(
        /decisionCapability|xox[baprs]-|secretValue|botToken|appToken/i
      );
      expect((history[0].data as JsonObject).definitionHash).toBe(
        'sha256:a02f094f7200f0e7e33bef7de2aba9b52638ac24adb9f017fd292764fbcb6988'
      );
    }

    const partial = (await Bun.file(
      join(directory, names[0])
    ).json()) as JsonObject[];
    expect(
      partial.filter(item => item.type === 'notification_delivery_succeeded')
    ).toHaveLength(1);
    expect(partial.some(item => item.type === 'run_failed')).toBe(false);

    const failed = (await Bun.file(
      join(directory, names[1])
    ).json()) as JsonObject[];
    expect(failed.at(-1)).toMatchObject({
      type: 'run_failed',
      data: {
        failureScope: 'notification',
        failure: { code: 'WOML_NOTIFICATION_DELIVERY_FAILED' },
      },
    });

    const approved = (await Bun.file(
      join(directory, names[2])
    ).json()) as JsonObject[];
    expect(
      approved.filter(item => item.type === 'notification_decision_accepted')
    ).toHaveLength(1);
    expect(
      approved.filter(item => item.type === 'approval_resolved')
    ).toHaveLength(1);
    expect(
      approved.filter(item => item.type === 'notification_message_updated')
    ).toHaveLength(2);
    for (const deliveryId of [
      'releaseApproval:notify:0:channel:0',
      'releaseApproval:notify:0:channel:1',
    ]) {
      const requested = approved.findIndex(
        item =>
          item.type === 'notification_message_update_requested' &&
          (item.data as JsonObject).deliveryId === deliveryId
      );
      const attempted = approved.findIndex(
        item =>
          item.type === 'notification_message_update_attempt_started' &&
          (item.data as JsonObject).deliveryId === deliveryId
      );
      expect(requested).toBeGreaterThan(-1);
      expect(attempted).toBeGreaterThan(requested);
    }
    expect(approved.at(-1)).toMatchObject({
      type: 'run_succeeded',
      data: { result: { decision: 'approved' } },
    });
  });

  test('pins every provider-host-v1 message and rejects resolved secrets', async () => {
    const { provider } = await validators();
    const directory = resolve(
      projectRoot,
      'woml-cli/tests/fixtures/notification-provider'
    );
    const names = (await readdir(directory))
      .filter(name => name.endsWith('.v1.json'))
      .sort();
    expect(names).toEqual([
      'deliver-unicode-crlf.v1.json',
      'deliver.v1.json',
      'delivery-success.v1.json',
      'failure.v1.json',
      'interaction.v1.json',
      'ready.v1.json',
      'update-success.v1.json',
      'update.v1.json',
    ]);

    for (const name of names) {
      const fixture = await readJson(join(directory, name));
      expect(provider(fixture), `${name}: ${errors(provider)}`).toBe(true);
      expect(JSON.stringify(fixture)).not.toMatch(
        /xox[baprs]-|secretValue|botTokenValue|appTokenValue|workflowContext/i
      );
    }

    const unicode = await readJson(
      join(directory, 'deliver-unicode-crlf.v1.json')
    );
    expect((unicode.message as JsonObject).approvalName).toBe(
      'Release café 🚜'
    );
    expect((unicode.message as JsonObject).approvalDescription).toBe(
      'First line\r\nSecond line'
    );

    const interaction = await readJson(join(directory, 'interaction.v1.json'));
    const competingInteraction = structuredClone(interaction);
    competingInteraction.interactionId = 'slack-envelope-02';
    competingInteraction.deliveryId = 'releaseApproval:notify:0:channel:1';
    competingInteraction.decision = 'rejected';
    competingInteraction.decisionCapability =
      'capability_competing_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789';
    expect(provider(interaction)).toBe(true);
    expect(provider(competingInteraction), errors(provider)).toBe(true);

    const deliver = await readJson(join(directory, 'deliver.v1.json'));
    const malformed = structuredClone(deliver);
    (malformed.credentials as JsonObject).botToken = {
      kind: 'literal',
      value: 'xoxb-forbidden',
    };
    expect(provider(malformed)).toBe(false);

    const completion = await readJson(
      join(directory, 'delivery-success.v1.json')
    );
    const secondCompletion = structuredClone(completion);
    secondCompletion.invocationId = 'notify-invocation-02';
    expect(provider(secondCompletion)).toBe(true);
    expect(provider(completion)).toBe(true);
  });
});
