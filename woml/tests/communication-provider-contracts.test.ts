import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import Ajv2020 from 'ajv/dist/2020';

const repositoryRoot = resolve(import.meta.dir, '../..');
const schemaRoot = resolve(repositoryRoot, 'docs/schemas');
const fixtureRoot = resolve(
  import.meta.dir,
  'fixtures/communication-providers'
);

const schemas = [
  'communication-trigger-payload.v1.schema.json',
  'communication-trigger-host.v1.schema.json',
  'communication-notification-adapter.v1.schema.json',
  'communication-messaging.v1.schema.json',
  'communication-provider-model.v1.schema.json',
  'communication-provider-run-event.v1.schema.json',
] as const;

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
  for (const name of schemas) {
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
  expect(validate(value), JSON.stringify(validate.errors)).toBe(true);
}

describe('Frozen communication-provider contracts', () => {
  test('validates every reviewed provider-neutral artifact', () => {
    const ajv = validators();
    const payloads = fixture('payloads.v1.json');
    const contracts = fixture('contracts.v1.json');

    for (const value of payloads) {
      expectValid(
        ajv,
        'https://woml.dev/schemas/communication-trigger-payload/v1',
        value
      );
    }
    for (const value of contracts.triggerHost) {
      expectValid(
        ajv,
        'https://woml.dev/schemas/communication-trigger-host/v1',
        value
      );
    }
    for (const value of contracts.notifications) {
      expectValid(
        ajv,
        'https://woml.dev/schemas/communication-notification-adapter/v1',
        value
      );
    }
    for (const value of contracts.messaging) {
      expectValid(
        ajv,
        'https://woml.dev/schemas/communication-messaging/v1',
        value
      );
    }
    for (const value of contracts.modelFragments) {
      expectValid(
        ajv,
        'https://woml.dev/schemas/communication-provider-model/v1',
        value
      );
    }
    for (const value of contracts.events) {
      expectValid(
        ajv,
        'https://woml.dev/schemas/communication-provider-run-event/v1',
        value
      );
    }
  });

  test('pins all version decisions and transport policies', () => {
    expect(fixture('semantics.v1.json')).toEqual({
      versions: {
        payload: 1,
        triggerHost: 1,
        notificationAdapter: 1,
        messaging: 1,
        model: 15,
        definitionPackage: 10,
        event: 14,
        store: 14,
        scriptHost: 8,
        capabilityCall: 1,
        presentation: 1,
      },
      providerOrder: ['telegram', 'discord', 'whatsapp'],
      transports: {
        telegram: 'long-polling',
        discord: 'gateway',
        whatsapp: 'official-cloud-api-webhook',
      },
      acknowledgement: 'durable-accepted-or-duplicate-only',
      localAliasResolution: 'explicit-local-wins-with-warning',
      approvalResolution: 'first-valid-durable-decision-wins',
      whatsappProactiveDelivery: 'approved-template-required',
      unsupportedV1: [
        'telegram-webhook',
        'discord-slash-command',
        'media',
        'edit',
        'delete',
        'raw-provider-json',
      ],
    });
  });

  test('rejects secret leakage, raw envelopes, invalid Slack compatibility, and unsafe WhatsApp text', () => {
    const ajv = validators();
    const payload = fixture('payloads.v1.json')[1];
    const validatePayload = ajv.getSchema(
      'https://woml.dev/schemas/communication-trigger-payload/v1'
    )!;
    expect(validatePayload({ ...payload, token: 'synthetic-secret' })).toBe(
      false
    );
    expect(
      validatePayload({
        ...payload,
        providerData: { raw: 'synthetic-envelope' },
      })
    ).toBe(false);
    expect(
      validatePayload({
        ...fixture('payloads.v1.json')[0],
        teamId: undefined,
      })
    ).toBe(false);

    const notification = fixture('contracts.v1.json').notifications[0];
    const validateNotification = ajv.getSchema(
      'https://woml.dev/schemas/communication-notification-adapter/v1'
    )!;
    expect(
      validateNotification({
        ...notification,
        provider: 'whatsapp',
        credentials: {
          accessToken: {
            kind: 'secretReference',
            name: 'WHATSAPP_ACCESS_TOKEN',
          },
          phoneNumberId: 'synthetic-phone-id',
        },
      })
    ).toBe(false);

    const messaging = fixture('contracts.v1.json').messaging[0];
    const validateMessaging = ajv.getSchema(
      'https://woml.dev/schemas/communication-messaging/v1'
    )!;
    expect(
      validateMessaging({
        ...messaging,
        credentials: { botToken: 'plaintext-is-forbidden' },
      })
    ).toBe(false);
  });

  test('allows multiplexed, out-of-order admission results by receipt identity', () => {
    const contracts = fixture('contracts.v1.json');
    const occurrence = contracts.triggerHost[1];
    const first = { ...occurrence, receiptId: 'receipt-1' };
    const second = {
      ...occurrence,
      receiptId: 'receipt-2',
      externalEventId: 'update-43',
    };
    const results = [
      {
        protocol: 'woml.communication-trigger-host',
        protocolVersion: 1,
        messageType: 'admission-result',
        receiptId: 'receipt-2',
        outcome: 'accepted',
        runId: 'run_2',
        duplicate: false,
      },
      {
        protocol: 'woml.communication-trigger-host',
        protocolVersion: 1,
        messageType: 'admission-result',
        receiptId: 'receipt-1',
        outcome: 'accepted',
        runId: 'run_1',
        duplicate: true,
      },
    ];
    const ajv = validators();
    for (const value of [first, second, ...results]) {
      expectValid(
        ajv,
        'https://woml.dev/schemas/communication-trigger-host/v1',
        value
      );
    }
    expect(results.map(item => item.receiptId)).toEqual([
      'receipt-2',
      'receipt-1',
    ]);
  });
});
