import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { readdir } from 'node:fs/promises';
import { join, resolve } from 'node:path';

import Ajv2020, { type ValidateFunction } from 'ajv/dist/2020';

import {
  analyzeWomlScript,
  compileWoml,
  parseWoml,
  WomlValidationError,
} from '../src';

type JsonObject = Record<string, unknown>;

const schemaDirectory = resolve(import.meta.dir, '../../docs/schemas');
const fixtureDirectory = resolve(
  import.meta.dir,
  'fixtures/services-contracts'
);

async function json(path: string): Promise<any> {
  return Bun.file(path).json();
}

async function validators() {
  const ajv = new Ajv2020({
    allErrors: true,
    strict: false,
    validateFormats: false,
  });
  const schemaNames = [
    'attempt-failure.v1.schema.json',
    'attempt-failure.v2.schema.json',
    'attempt-failure.v3.schema.json',
    ...Array.from(
      { length: 8 },
      (_, index) => `compiled-workflow-model.v${index + 1}.schema.json`
    ),
    ...Array.from(
      { length: 8 },
      (_, index) => `run-event.v${index + 1}.schema.json`
    ),
    'script-host-protocol.v3.schema.json',
    'capability-call.v1.schema.json',
    'service-progress.v1.schema.json',
    'native-fetch-observation.v1.schema.json',
    'managed-http.v1.schema.json',
    'script-host-protocol.v4.schema.json',
  ];
  for (const name of schemaNames)
    ajv.addSchema(await json(join(schemaDirectory, name)));
  const get = (id: string): ValidateFunction => {
    const validator = ajv.getSchema(id);
    if (validator === undefined) throw new Error(`Missing schema ${id}`);
    return validator;
  };
  return {
    model: get('https://cronflow.dev/schemas/compiled-workflow-model/v8'),
    event: get('https://cronflow.dev/schemas/run-event/v8'),
    capability: get('https://cronflow.dev/schemas/capability-call/v1'),
    progress: get('https://cronflow.dev/schemas/service-progress/v1'),
    fetch: get('https://cronflow.dev/schemas/native-fetch-observation/v1'),
    http: get('https://cronflow.dev/schemas/managed-http/v1'),
    host: get('https://cronflow.dev/schemas/script-host-protocol/v4'),
  };
}

const schemas = await validators();

function workflow(script: string): string {
  return `<workflow id="analysis-contract" version="1.0.0">
  <triggers><manual id="start" /></triggers>
  <steps><step id="operation"><script>${script}</script></step></steps>
</workflow>`;
}

interface OperationState {
  readonly runId: string;
  readonly invocationId: string;
  readonly nodeId: string;
  readonly attemptNumber: number;
  readonly capability: string;
  readonly operation: string;
  readonly executionMode: string;
  readonly operationKey: string;
  terminal: boolean;
}

function inspectOperationHistory(
  events: readonly JsonObject[]
): readonly string[] {
  const issues: string[] = [];
  const starts = new Map<string, OperationState>();
  let runId: string | undefined;
  let sequence = 0;
  for (const event of events) {
    const currentRun = String(event.runId);
    if (runId === undefined) runId = currentRun;
    if (currentRun !== runId) issues.push('cross-run event');
    if (event.sequence !== sequence + 1) issues.push('non-contiguous sequence');
    sequence = Number(event.sequence);
    const type = String(event.type);
    const data = event.data as JsonObject;
    if (!type.startsWith('operation_')) {
      if (type === 'step_attempt_succeeded') {
        for (const state of starts.values()) {
          if (!state.terminal && state.executionMode === 'managed') {
            issues.push('step succeeded with active managed operation');
          }
        }
      }
      continue;
    }
    const key = `${currentRun}\0${String(data.invocationId)}\0${String(data.callId)}`;
    if (type === 'operation_started') {
      if (starts.has(key)) issues.push('duplicate operation start');
      starts.set(key, {
        runId: currentRun,
        invocationId: String(data.invocationId),
        nodeId: String(data.nodeId),
        attemptNumber: Number(data.attemptNumber),
        capability: String(data.capability),
        operation: String(data.operation),
        executionMode: String(data.executionMode),
        operationKey: String(data.operationKey),
        terminal: false,
      });
      continue;
    }
    const start = starts.get(key);
    if (start === undefined) {
      issues.push('terminal operation without start');
      continue;
    }
    if (start.terminal) issues.push('duplicate terminal operation');
    for (const field of [
      'nodeId',
      'attemptNumber',
      'capability',
      'operation',
      'executionMode',
      'operationKey',
    ] as const) {
      if (String(start[field]) !== String(data[field]))
        issues.push(`terminal ${field} mismatch`);
    }
    start.terminal = true;
  }
  return issues;
}

describe('SC0 frozen service and capability contracts', () => {
  test('all reviewed standalone fixtures validate against their versioned schema', async () => {
    const names = (await readdir(fixtureDirectory))
      .filter(name => name.endsWith('.json'))
      .sort();
    expect(names.length).toBe(13);
    for (const name of names) {
      const fixture = await json(join(fixtureDirectory, name));
      if (name === 'script-host-messages.v4.json') {
        for (const message of fixture) {
          expect(
            schemas.host(message),
            `${name}: ${JSON.stringify(schemas.host.errors)}`
          ).toBe(true);
          if (message.call !== undefined) {
            expect(message.invocationId).toBe(message.call.invocationId);
            expect(message.callId).toBe(message.call.callId);
          }
          if (message.result !== undefined) {
            expect(message.invocationId).toBe(message.result.invocationId);
            expect(message.callId).toBe(message.result.callId);
          }
          if (message.observation !== undefined) {
            expect(message.invocationId).toBe(message.observation.invocationId);
            expect(message.requestId).toBe(message.observation.requestId);
          }
        }
        const callIds = fixture
          .filter(
            (message: JsonObject) => message.messageType === 'capability_call'
          )
          .map((message: JsonObject) => message.callId);
        const resultIds = fixture
          .filter(
            (message: JsonObject) => message.messageType === 'capability_result'
          )
          .map((message: JsonObject) => message.callId);
        expect(callIds).toEqual(['call-http-a', 'call-http-b']);
        expect(resultIds).toEqual(['call-http-b', 'call-http-a']);
        continue;
      }
      const validator = name.startsWith('capability-')
        ? schemas.capability
        : name.startsWith('service-progress-')
          ? schemas.progress
          : name.startsWith('native-fetch-')
            ? schemas.fetch
            : schemas.http;
      expect(
        validator(fixture),
        `${name}: ${JSON.stringify(validator.errors)}`
      ).toBe(true);
    }
  });

  test('validates Model v8 and the complete operation history fixture', async () => {
    const model = await json(
      resolve(import.meta.dir, 'fixtures/services-bindings.compiled.v8.json')
    );
    expect(schemas.model(model), JSON.stringify(schemas.model.errors)).toBe(
      true
    );
    const events = await json(
      resolve(
        import.meta.dir,
        'fixtures/run-events/services-http.events.v8.json'
      )
    );
    for (const event of events) {
      expect(schemas.event(event), JSON.stringify(schemas.event.errors)).toBe(
        true
      );
    }
    expect(inspectOperationHistory(events)).toEqual([]);
    const failedEvents = await json(
      resolve(
        import.meta.dir,
        'fixtures/run-events/services-http-failed.events.v8.json'
      )
    );
    for (const event of failedEvents) {
      expect(schemas.event(event), JSON.stringify(schemas.event.errors)).toBe(
        true
      );
    }
    expect(inspectOperationHistory(failedEvents)).toEqual([]);
  });

  test('rejects malformed, cross-run, duplicate, and unclosed managed histories', async () => {
    const events = await json(
      resolve(
        import.meta.dir,
        'fixtures/run-events/services-http.events.v8.json'
      )
    );
    const duplicate = [
      ...events.slice(0, 4),
      { ...events[3], eventId: 'duplicate', sequence: 5 },
      ...events
        .slice(4)
        .map((event: JsonObject) => ({
          ...event,
          sequence: Number(event.sequence) + 1,
        })),
    ];
    expect(inspectOperationHistory(duplicate)).toContain(
      'duplicate terminal operation'
    );
    const crossed = events.map((event: JsonObject, index: number) =>
      index === 3 ? { ...event, runId: 'another-run' } : event
    );
    expect(inspectOperationHistory(crossed)).toContain('cross-run event');
    const unclosed = events.filter(
      (event: JsonObject) => event.type !== 'operation_succeeded'
    );
    expect(inspectOperationHistory(unclosed)).toContain(
      'step succeeded with active managed operation'
    );
    expect(
      schemas.event({
        ...events[2],
        data: { ...(events[2].data as JsonObject), authorization: 'forbidden' },
      })
    ).toBe(false);
  });

  test('pins size failures, redaction, HTTP body exclusivity, and protocol correlation slots', async () => {
    const failure = await json(
      join(fixtureDirectory, 'capability-failed.v1.json')
    );
    for (const kind of [
      'input_too_large',
      'result_too_large',
      'frame_too_large',
    ]) {
      expect(
        schemas.capability({ ...failure, error: { ...failure.error, kind } })
      ).toBe(true);
    }
    const progress = await json(
      join(fixtureDirectory, 'service-progress-started.v1.json')
    );
    expect(
      schemas.progress({ ...progress, metadata: { authorization: 'never' } })
    ).toBe(false);
    const observed = await json(
      join(fixtureDirectory, 'native-fetch-started.v1.json')
    );
    expect(
      schemas.fetch({
        ...observed,
        origin: 'https://user:password@api.example.test',
      })
    ).toBe(false);
    const http = await json(
      join(fixtureDirectory, 'managed-http-request.v1.json')
    );
    expect(schemas.http({ ...http, text: 'also a body' })).toBe(false);
    const host = await json(
      join(fixtureDirectory, 'script-host-messages.v4.json')
    );
    const call = host.find(
      (message: JsonObject) => message.messageType === 'capability_call'
    );
    expect(schemas.host({ ...call, unexpected: true })).toBe(false);
  });

  test('frames multibyte UTF-8 and literal CRLF by bytes rather than JS characters', async () => {
    const messages = await json(
      join(fixtureDirectory, 'script-host-messages.v4.json')
    );
    const execute = messages.find(
      (message: JsonObject) => message.messageType === 'execute'
    );
    const payload = JSON.stringify(execute);
    expect(payload).toContain('Héllo 🌍\\r\\nsecond line');
    expect(Buffer.byteLength(payload, 'utf8')).toBeGreaterThan(payload.length);
    const frame = `Content-Length: ${Buffer.byteLength(payload, 'utf8')}\r\n\r\n${payload}`;
    const [header, body] = frame.split('\r\n\r\n', 2);
    expect(Number(header.slice('Content-Length: '.length))).toBe(
      Buffer.byteLength(body, 'utf8')
    );
    expect(JSON.parse(body).context.trigger.note).toContain('\r\n');
  });
});

describe('SC1 script analysis and Model v8 lowering', () => {
  test('lowers the reviewed WOML fixture exactly and records names only', async () => {
    const source = readFileSync(
      resolve(import.meta.dir, 'fixtures/services-bindings.woml'),
      'utf8'
    );
    const expected = await json(
      resolve(import.meta.dir, 'fixtures/services-bindings.compiled.v8.json')
    );
    const compiled = compileWoml(
      parseWoml(source, { file: 'services-bindings.woml' })
    );
    expect(compiled).toEqual(expected);
    expect(JSON.stringify(compiled)).not.toContain('resolved-in-memory');
  });

  test('discovers, sorts, and deduplicates only literal secret reads', () => {
    const analysis = analyzeWomlScript(`
      // secrets.COMMENT_ONLY
      const text = "secrets.STRING_ONLY";
      return [secrets.ZETA_TOKEN, secrets.ALPHA_TOKEN, secrets.ZETA_TOKEN, text];
    `);
    expect(analysis).toEqual({
      requiredSecrets: ['ALPHA_TOKEN', 'ZETA_TOKEN'],
      usesServices: false,
      usesNativeFetch: false,
    });
  });

  test('selects Model v8 for services or native Fetch and preserves Model v1 otherwise', () => {
    expect(
      compileWoml(parseWoml(workflow('return services.cache.get("key");')))
        .schemaVersion
    ).toBe(8);
    expect(
      compileWoml(parseWoml(workflow('return fetch("https://example.test");')))
        .schemaVersion
    ).toBe(8);
    expect(
      compileWoml(
        parseWoml(
          workflow(
            'const request = fetch; return request("https://example.test");'
          )
        )
      ).schemaVersion
    ).toBe(8);
    expect(
      compileWoml(
        parseWoml(
          workflow('return globalThis["fetch"]("https://example.test");')
        )
      ).schemaVersion
    ).toBe(8);
    expect(
      compileWoml(parseWoml(workflow('return { ok: true };'))).schemaVersion
    ).toBe(1);
  });

  test('rejects computed, whole-object, optional, invalid, and shadowed binding use with source locations', () => {
    const cases = [
      ['return secrets[name];', 'WOML_SCRIPT_SECRET_ACCESS_DYNAMIC'],
      ['return Object.keys(secrets);', 'WOML_SCRIPT_SECRET_ACCESS_UNSUPPORTED'],
      ['return secrets?.API_TOKEN;', 'WOML_SCRIPT_SECRET_ACCESS_UNSUPPORTED'],
      ['return secrets.apiToken;', 'WOML_SCRIPT_SECRET_NAME_INVALID'],
      [
        'return services["http"].request({});',
        'WOML_SCRIPT_SERVICE_ACCESS_DYNAMIC',
      ],
      [
        'return services.http["request"]({});',
        'WOML_SCRIPT_SERVICE_ACCESS_DYNAMIC',
      ],
      [
        'services.http = {}; return true;',
        'WOML_SCRIPT_SERVICE_WRITE_UNSUPPORTED',
      ],
      [
        'secrets.API_TOKEN = "changed"; return true;',
        'WOML_SCRIPT_SECRET_WRITE_UNSUPPORTED',
      ],
      [
        'function nested(services) { return services; } return nested({});',
        'WOML_SCRIPT_BINDING_SHADOWED',
      ],
      [
        'const fetch = () => null; return fetch();',
        'WOML_SCRIPT_BINDING_SHADOWED',
      ],
      [
        'globalThis.fetch = () => null; return true;',
        'WOML_SCRIPT_FETCH_WRITE_UNSUPPORTED',
      ],
      [
        'return import("./network.js");',
        'WOML_SCRIPT_DYNAMIC_IMPORT_UNSUPPORTED',
      ],
      ['return require("node:net");', 'WOML_SCRIPT_MODULE_REQUIRE_UNSUPPORTED'],
      [
        'return Bun.connect({ hostname: "localhost", port: 80 });',
        'WOML_SCRIPT_RAW_NETWORK_UNSUPPORTED',
      ],
      [
        'return new WebSocket("ws://localhost");',
        'WOML_SCRIPT_RAW_NETWORK_UNSUPPORTED',
      ],
    ] as const;
    for (const [script, code] of cases) {
      try {
        compileWoml(
          parseWoml(workflow(script), { file: 'invalid-bindings.woml' })
        );
        throw new Error('Expected validation failure');
      } catch (error) {
        expect(error).toBeInstanceOf(WomlValidationError);
        const diagnostic = (error as WomlValidationError).diagnostic;
        expect(diagnostic.code).toBe(code);
        expect(diagnostic.file).toBe('invalid-bindings.woml');
        expect(diagnostic.location.start.line).toBeGreaterThan(0);
        expect(diagnostic.location.start.column).toBeGreaterThan(0);
      }
    }
  });

  test('reports JavaScript syntax failures before lowering', () => {
    expect(() =>
      compileWoml(parseWoml(workflow('return { broken: ; }')))
    ).toThrow(WomlValidationError);
    try {
      compileWoml(parseWoml(workflow('return { broken: ; }')));
    } catch (error) {
      expect((error as WomlValidationError).diagnostic.code).toBe(
        'WOML_SCRIPT_SYNTAX_INVALID'
      );
    }
  });
});
