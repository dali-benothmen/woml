import { describe, expect, test } from 'bun:test';
import { readdirSync } from 'node:fs';
import { resolve } from 'node:path';

import Ajv2020, { type ValidateFunction } from 'ajv/dist/2020';
import addFormats from 'ajv-formats';

type JsonObject = Record<string, unknown>;

const projectRoot = resolve(import.meta.dir, '../..');
const protocolFixtureDirectory = resolve(
  import.meta.dir,
  'fixtures/script-host',
);
const runEventFixtureDirectory = resolve(
  projectRoot,
  'woml/tests/fixtures/run-events',
);

async function readJson(path: string): Promise<unknown> {
  return JSON.parse(await Bun.file(path).text());
}

async function schema(name: string): Promise<JsonObject> {
  return (await readJson(
    resolve(projectRoot, 'docs/schemas', name),
  )) as JsonObject;
}

function validationMessage(validate: ValidateFunction): string {
  return JSON.stringify(validate.errors, null, 2);
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
    .map(
      (key) => `${JSON.stringify(key)}:${canonicalizeJson(object[key])}`,
    )
    .join(',')}}`;
}

function definitionHash(value: unknown): string {
  const hexadecimal = new Bun.CryptoHasher('sha256')
    .update(canonicalizeJson(value))
    .digest('hex');
  return `sha256:${hexadecimal}`;
}

async function validators(): Promise<{
  readonly failure: ValidateFunction;
  readonly protocol: ValidateFunction;
  readonly event: ValidateFunction;
}> {
  const ajv = new Ajv2020({
    allErrors: true,
    strict: false,
  });
  addFormats(ajv);
  const failureSchema = await schema('attempt-failure.v1.schema.json');
  const protocolSchema = await schema('script-host-protocol.v1.schema.json');
  const eventSchema = await schema('run-event.v1.schema.json');
  ajv.addSchema(failureSchema);

  return {
    failure: ajv.getSchema(
      'https://cronflow.dev/schemas/attempt-failure/v1',
    )!,
    protocol: ajv.compile(protocolSchema),
    event: ajv.compile(eventSchema),
  };
}

function encodeFrame(message: unknown): Buffer {
  const body = Buffer.from(JSON.stringify(message), 'utf8');
  const header = Buffer.from(`Content-Length: ${body.byteLength}\r\n\r\n`, 'ascii');
  return Buffer.concat([header, body]);
}

class ReferenceFrameDecoder {
  #buffer = Buffer.alloc(0);

  push(chunk: Uint8Array): unknown[] {
    this.#buffer = Buffer.concat([this.#buffer, chunk]);
    const decoded: unknown[] = [];

    while (true) {
      const headerEnd = this.#buffer.indexOf('\r\n\r\n');
      if (headerEnd < 0) break;

      const header = this.#buffer.subarray(0, headerEnd).toString('ascii');
      const match = /^Content-Length: ([0-9]+)$/.exec(header);
      if (match === null) throw new Error(`Invalid frame header: ${header}`);
      const length = Number(match[1]);
      const bodyStart = headerEnd + 4;
      const frameEnd = bodyStart + length;
      if (this.#buffer.byteLength < frameEnd) break;

      decoded.push(
        JSON.parse(this.#buffer.subarray(bodyStart, frameEnd).toString('utf8')),
      );
      this.#buffer = this.#buffer.subarray(frameEnd);
    }

    return decoded;
  }

  get remainingBytes(): number {
    return this.#buffer.byteLength;
  }
}

describe('script-host protocol v1 contract', () => {
  test('all reviewed protocol fixtures validate', async () => {
    const { protocol } = await validators();
    const fixtureNames = readdirSync(protocolFixtureDirectory)
      .filter((name) => name.endsWith('.json'))
      .sort();

    expect(fixtureNames).toEqual([
      'context-too-large.v1.json',
      'execute.v1.json',
      'non-json.v1.json',
      'ready.v1.json',
      'result-too-large.v1.json',
      'success.v1.json',
      'thrown.v1.json',
      'timeout.v1.json',
      'unicode-crlf.execute.v1.json',
      'worker-crashed.v1.json',
    ]);

    for (const fixtureName of fixtureNames) {
      const fixture = await readJson(
        resolve(protocolFixtureDirectory, fixtureName),
      );
      expect(
        protocol(fixture),
        `${fixtureName}: ${validationMessage(protocol)}`,
      ).toBe(true);
    }
  });

  test('code and kind pairs are inseparable', async () => {
    const { failure } = await validators();
    const mismatched = {
      kind: 'script_timed_out',
      code: 'WOML_SCRIPT_THROWN',
      message: 'wrong pair',
    };

    expect(failure(mismatched)).toBe(false);
  });

  test('host responses cannot claim host-crashed or interrupted failures', async () => {
    const { protocol } = await validators();
    const response = (await readJson(
      resolve(protocolFixtureDirectory, 'thrown.v1.json'),
    )) as JsonObject;
    const outcome = structuredClone(response.outcome) as JsonObject;
    outcome.error = {
      kind: 'host_crashed',
      code: 'WOML_SCRIPT_HOST_CRASHED',
      message: 'A dead host cannot report this.',
    };

    expect(protocol({ ...response, outcome })).toBe(false);
  });

  test('unsupported protocol versions and undocumented fields fail closed', async () => {
    const { protocol } = await validators();
    const execute = (await readJson(
      resolve(protocolFixtureDirectory, 'execute.v1.json'),
    )) as JsonObject;

    expect(protocol({ ...execute, protocolVersion: 2 })).toBe(false);
    expect(protocol({ ...execute, services: {} })).toBe(false);
  });

  test('UTF-8 and literal CRLF values survive exact byte framing', async () => {
    const message = await readJson(
      resolve(protocolFixtureDirectory, 'unicode-crlf.execute.v1.json'),
    );
    const source = (message as { source: string }).source;
    const note = (message as { context: { trigger: { note: string } } }).context
      .trigger.note;
    expect(source).toContain('\r\n');
    expect(note).toBe('first line\r\nsecond line');

    const body = JSON.stringify(message);
    const encodedBodyLength = new TextEncoder().encode(body).byteLength;
    expect(encodedBodyLength).toBeGreaterThan(body.length);

    const frame = encodeFrame(message);
    const headerEnd = frame.indexOf('\r\n\r\n');
    expect(frame.subarray(0, headerEnd).toString('ascii')).toBe(
      `Content-Length: ${encodedBodyLength}`,
    );

    const decoder = new ReferenceFrameDecoder();
    const decoded: unknown[] = [];
    for (const byte of frame) {
      decoded.push(...decoder.push(Uint8Array.of(byte)));
    }
    expect(decoded).toEqual([message]);
    expect(decoder.remainingBytes).toBe(0);
  });

  test('combined frames preserve out-of-order invocation correlation', async () => {
    const success = (await readJson(
      resolve(protocolFixtureDirectory, 'success.v1.json'),
    )) as JsonObject;
    const completions = [
      { ...success, invocationId: 'inv_02' },
      { ...success, invocationId: 'inv_01' },
      { ...success, invocationId: 'inv_03' },
    ];
    const combined = Buffer.concat(completions.map(encodeFrame));
    const decoder = new ReferenceFrameDecoder();

    expect(decoder.push(combined)).toEqual(completions);
    expect(decoder.remainingBytes).toBe(0);
  });
});

describe('run-event v1 contract', () => {
  test('the hello event log and recovery failures validate', async () => {
    const { event } = await validators();
    const hello = (await readJson(
      resolve(runEventFixtureDirectory, 'hello.events.v1.json'),
    )) as unknown[];
    const failureFixtures = [
      'host-crashed.event.v1.json',
      'interrupted.event.v1.json',
    ];

    for (const [index, runEvent] of hello.entries()) {
      expect(
        event(runEvent),
        `hello event ${index + 1}: ${validationMessage(event)}`,
      ).toBe(true);
    }
    for (const fixtureName of failureFixtures) {
      const fixture = await readJson(
        resolve(runEventFixtureDirectory, fixtureName),
      );
      expect(
        event(fixture),
        `${fixtureName}: ${validationMessage(event)}`,
      ).toBe(true);
    }
  });

  test('the reviewed hello log obeys ordering, attempt, and fold invariants', async () => {
    const events = (await readJson(
      resolve(runEventFixtureDirectory, 'hello.events.v1.json'),
    )) as Array<{
      eventId: string;
      runId: string;
      sequence: number;
      type: string;
      data: JsonObject;
    }>;

    expect(events[0].type).toBe('run_started');
    expect(events.at(-1)?.type).toBe('run_succeeded');
    expect(events.map((event) => event.sequence)).toEqual([1, 2, 3, 4, 5, 6]);
    expect(new Set(events.map((event) => event.eventId)).size).toBe(events.length);
    expect(new Set(events.map((event) => event.runId)).size).toBe(1);

    const started = new Set<string>();
    const context: { trigger: JsonObject; steps: JsonObject } = {
      trigger: {},
      steps: {},
    };
    for (const event of events) {
      if (event.type === 'run_started') {
        context.trigger = event.data.trigger as JsonObject;
      } else if (event.type === 'step_attempt_started') {
        started.add(String(event.data.invocationId));
      } else if (event.type === 'step_attempt_succeeded') {
        expect(started.has(String(event.data.invocationId))).toBe(true);
        context.steps[String(event.data.nodeId)] = event.data.output;
      }
    }

    expect(context).toEqual({
      trigger: {},
      steps: {
        a: { x: 'World' },
        b: { message: 'Hello World' },
      },
    });
  });

  test('run_started binds to the RFC 8785 SHA-256 hash of the compiled fixture', async () => {
    const compiled = await readJson(
      resolve(projectRoot, 'woml/tests/fixtures/hello.compiled.v1.json'),
    );
    const events = (await readJson(
      resolve(runEventFixtureDirectory, 'hello.events.v1.json'),
    )) as Array<{ type: string; data: JsonObject }>;
    const started = events.find((event) => event.type === 'run_started');

    expect(started?.data.definitionHash).toBe(definitionHash(compiled));
    expect(started?.data.definitionHash).toBe(
      'sha256:97788d011d2306b254e9ab36ec9262887517a682357a955d770242774317939a',
    );
  });

  test('invalid RFC 3339 timestamps fail schema validation', async () => {
    const { event } = await validators();
    const events = (await readJson(
      resolve(runEventFixtureDirectory, 'hello.events.v1.json'),
    )) as JsonObject[];

    expect(event({ ...events[0], occurredAt: 'not-a-timestamp' })).toBe(false);
  });

  test('reserved future event names are rejected by the current executable schema', async () => {
    const { event } = await validators();
    const started = (await readJson(
      resolve(runEventFixtureDirectory, 'hello.events.v1.json'),
    )) as JsonObject[];

    expect(event({ ...started[0], type: 'branch_selected' })).toBe(false);
  });
});
