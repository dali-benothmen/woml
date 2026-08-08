import { describe, expect, test } from 'bun:test';
import { readdir } from 'node:fs/promises';
import { resolve } from 'node:path';

import Ajv2020 from 'ajv/dist/2020';

const fixturesDirectory = resolve(
  import.meta.dir,
  'fixtures/event-contracts'
);
const publicationSchema = await Bun.file(
  new URL(
    '../../docs/schemas/event-publication.v1.schema.json',
    import.meta.url
  )
).json();
const httpSchema = await Bun.file(
  new URL(
    '../../docs/schemas/event-publisher-http.v1.schema.json',
    import.meta.url
  )
).json();
const ajv = new Ajv2020({
  allErrors: true,
  strict: false,
  validateFormats: false,
});
const validatePublication = ajv.compile(publicationSchema);
const validateHttp = ajv.compile(httpSchema);

describe('T11 Event Publication v1 contracts', () => {
  test('accepts every reviewed publish, fan-out, duplicate, and rejection fixture', async () => {
    const names = (await readdir(fixturesDirectory))
      .filter(name => name.endsWith('.json'))
      .sort();
    expect(names).toEqual([
      'publish.v1.json',
      'published-all.v1.json',
      'published-duplicate.v1.json',
      'published-partial.v1.json',
      'published-rejected.v1.json',
      'rejected-unauthorized.v1.json',
    ]);
    for (const name of names) {
      const fixture = await Bun.file(resolve(fixturesDirectory, name)).json();
      expect(
        validatePublication(fixture),
        `${name}: ${JSON.stringify(validatePublication.errors)}`
      ).toBe(true);
    }
  });

  test('pins collision-safe per-subscriber Trigger Ingress identities', () => {
    expect(publicationSchema['x-trigger-source-identity']).toBe(
      'event:v1:sha256:<hex SHA-256 of UTF-8(eventId + NUL + workflowId + NUL + triggerId)>'
    );
    const identity = (workflowId: string, triggerId: string) => {
      const material = `order-42-created\0${workflowId}\0${triggerId}`;
      const digest = new Bun.CryptoHasher('sha256')
        .update(material)
        .digest('hex');
      return `event:v1:sha256:${digest}`;
    };
    expect(identity('send-confirmation', 'orderCreated')).toBe(
      'event:v1:sha256:7ee0e7eb7ec91d3dfdcbbf57e3a601942038e1fe13e9eaf64f1dc038c69fa8d8'
    );
    expect(identity('update-inventory', 'orderCreated')).toBe(
      'event:v1:sha256:2f09547347430c4238c9c9257ebe485feff53cff67f58f5163fe4e8e03385e76'
    );
  });

  test('requires fan-out status to agree with all subscriber results', async () => {
    const all = await Bun.file(
      resolve(fixturesDirectory, 'published-all.v1.json')
    ).json();
    expect(validatePublication({ ...all, status: 'rejected' })).toBe(false);

    const partial = await Bun.file(
      resolve(fixturesDirectory, 'published-partial.v1.json')
    ).json();
    expect(validatePublication({ ...partial, status: 'accepted' })).toBe(false);
    expect(validatePublication({ ...partial, status: 'rejected' })).toBe(false);
  });

  test('pins the authenticated HTTP boundary and public response shape', async () => {
    expect(httpSchema['x-http-request']).toEqual({
      method: 'POST',
      path: '/_woml/events/{eventName}',
      requiredHeaders: [
        'Authorization: Bearer <control-token>',
        'Event-ID: <event-id>',
        'Content-Type: application/json',
      ],
      body: 'One top-level JSON object, limited to 1 MiB',
    });
    const logical = await Bun.file(
      resolve(fixturesDirectory, 'published-partial.v1.json')
    ).json();
    const response = {
      eventId: logical.eventId,
      eventName: logical.eventName,
      status: logical.status,
      deliveries: logical.deliveries.map(
        ({ occurrenceId: _occurrenceId, ...delivery }: Record<string, unknown>) =>
          delivery
      ),
    };
    expect(
      validateHttp(response),
      JSON.stringify(validateHttp.errors)
    ).toBe(true);
  });

  test('rejects credentials and unknown publisher fields from every durable contract', async () => {
    const publish = await Bun.file(
      resolve(fixturesDirectory, 'publish.v1.json')
    ).json();
    expect(
      validatePublication({
        ...publish,
        authorization: 'Bearer must-never-cross-this-boundary',
      })
    ).toBe(false);
    expect(JSON.stringify(publicationSchema)).not.toContain('control-token');
    for (const name of await readdir(fixturesDirectory)) {
      const text = await Bun.file(resolve(fixturesDirectory, name)).text();
      expect(text).not.toContain('Authorization');
      expect(text).not.toContain('Bearer ');
      expect(text).not.toContain('secret');
    }
  });
});
