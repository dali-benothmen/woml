import { describe, expect, test } from 'bun:test';
import { readdirSync } from 'node:fs';
import { basename, join, resolve } from 'node:path';

import Ajv2020, { type ValidateFunction } from 'ajv/dist/2020';
import addFormats from 'ajv-formats';
import { compileWoml, parseWoml, type CompiledWorkflowDefinition } from 'woml';
import { parseExecutionProgress } from '../src/rust-executor';

type JsonObject = Record<string, unknown>;

const projectRoot = resolve(import.meta.dir, '../..');
const schemaDirectory = resolve(projectRoot, 'docs/schemas');
const womlFixtureDirectory = resolve(projectRoot, 'woml/tests/fixtures');
const eventFixtureDirectory = join(womlFixtureDirectory, 'run-events');
const hostFixtureDirectory = resolve(
  import.meta.dir,
  'fixtures/script-host-v3'
);
const progressFixtureDirectory = resolve(
  import.meta.dir,
  'fixtures/execution-progress-v1'
);

async function readJson(path: string): Promise<unknown> {
  return JSON.parse(await Bun.file(path).text());
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
    .map(key => `${JSON.stringify(key)}:${canonicalizeJson(object[key])}`)
    .join(',')}}`;
}

function sha256Canonical(value: unknown): string {
  return `sha256:${new Bun.CryptoHasher('sha256')
    .update(canonicalizeJson(value))
    .digest('hex')}`;
}

async function retryValidators(): Promise<{
  readonly model: ValidateFunction;
  readonly event: ValidateFunction;
  readonly host: ValidateFunction;
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
    'run-event.v1.schema.json',
    'run-event.v2.schema.json',
    'run-event.v3.schema.json',
    'run-event.v4.schema.json',
    'run-event.v5.schema.json',
    'run-event.v6.schema.json',
    'script-host-protocol.v3.schema.json',
    'execution-progress.v1.schema.json',
  ];
  for (const name of names) {
    ajv.addSchema((await readJson(join(schemaDirectory, name))) as JsonObject);
  }

  return {
    model: ajv.getSchema(
      'https://cronflow.dev/schemas/compiled-workflow-model/v6'
    )!,
    event: ajv.getSchema('https://cronflow.dev/schemas/run-event/v6')!,
    host: ajv.getSchema(
      'https://cronflow.dev/schemas/script-host-protocol/v3'
    )!,
    progress: ajv.getSchema(
      'https://cronflow.dev/schemas/execution-progress/v1'
    )!,
  };
}

function historyIssues(history: unknown): string[] {
  if (!Array.isArray(history) || history.length === 0) {
    return ['History must be a non-empty event array.'];
  }
  const issues: string[] = [];
  const first = history[0] as JsonObject;
  const runId = first.runId;
  const effectKeys = new Map<string, string>();
  const active = new Map<string, { attempt: number; invocationId: string }>();
  const scheduled = new Map<
    string,
    { nextAttempt: number; scheduledAt: string }
  >();

  if (first.type !== 'run_started') issues.push('run_started must be first.');
  for (const [index, rawEvent] of history.entries()) {
    const event = rawEvent as JsonObject;
    const data = event.data as JsonObject;
    if (event.eventSchemaVersion !== 6)
      issues.push(`Event ${index + 1} is not schema v6.`);
    if (event.runId !== runId) issues.push(`Event ${index + 1} changes runId.`);
    if (event.sequence !== index + 1)
      issues.push(`Event ${index + 1} has a non-contiguous sequence.`);

    if (event.type === 'step_attempt_started') {
      const nodeId = String(data.nodeId);
      const attempt = Number(data.attempt);
      const invocationId = String(data.invocationId);
      const priorSchedule = scheduled.get(nodeId);
      if (attempt > 1) {
        if (priorSchedule?.nextAttempt !== attempt) {
          issues.push(`${nodeId} attempt ${attempt} has no matching schedule.`);
        } else if (
          Date.parse(String(event.occurredAt)) <
          Date.parse(priorSchedule.scheduledAt)
        ) {
          issues.push(
            `${nodeId} attempt ${attempt} starts before its schedule.`
          );
        }
        scheduled.delete(nodeId);
      }
      if (active.has(nodeId)) issues.push(`${nodeId} has two active attempts.`);
      active.set(nodeId, { attempt, invocationId });
      const key = String(data.idempotencyKey);
      const priorKey = effectKeys.get(nodeId);
      if (priorKey !== undefined && priorKey !== key) {
        issues.push(`${nodeId} changes its idempotency key.`);
      }
      effectKeys.set(nodeId, key);
    }

    if (
      event.type === 'step_attempt_failed' ||
      event.type === 'step_attempt_succeeded'
    ) {
      const nodeId = String(data.nodeId);
      const current = active.get(nodeId);
      if (
        current === undefined ||
        current.attempt !== data.attempt ||
        current.invocationId !== data.invocationId
      ) {
        issues.push(
          `${nodeId} terminal event does not match its active attempt.`
        );
      }
      active.delete(nodeId);
    }

    if (event.type === 'step_retry_scheduled') {
      const previous = history[index - 1] as JsonObject | undefined;
      const previousData = previous?.data as JsonObject | undefined;
      if (
        previous?.type !== 'step_attempt_failed' ||
        previousData?.nodeId !== data.nodeId ||
        previousData?.attempt !== data.failedAttempt
      ) {
        issues.push(
          'Retry schedule must immediately follow its failed attempt.'
        );
      }
      const failure = previousData?.failure as JsonObject | undefined;
      if (failure?.kind !== 'script_threw') {
        issues.push('Only script_threw may schedule a retry.');
      }
      if (Number(data.nextAttempt) !== Number(data.failedAttempt) + 1) {
        issues.push('nextAttempt must equal failedAttempt + 1.');
      }
      scheduled.set(String(data.nodeId), {
        nextAttempt: Number(data.nextAttempt),
        scheduledAt: String(data.scheduledAt),
      });
    }
  }
  return issues;
}

function encodeFrame(message: unknown): Buffer {
  const body = Buffer.from(JSON.stringify(message), 'utf8');
  return Buffer.concat([
    Buffer.from(`Content-Length: ${body.byteLength}\r\n\r\n`, 'ascii'),
    body,
  ]);
}

describe('Retry and idempotency contracts', () => {
  test('the CLI strictly decodes every frozen progress message', async () => {
    for (const name of readdirSync(progressFixtureDirectory).sort()) {
      const json = await Bun.file(join(progressFixtureDirectory, name)).text();
      const progress = parseExecutionProgress(json);
      expect('contract' in progress ? progress.contract : undefined).toBe(
        'woml.execution-progress'
      );
    }
    const failed = (await readJson(
      join(progressFixtureDirectory, 'attempt-failed.v1.json')
    )) as JsonObject;
    expect(() =>
      parseExecutionProgress(
        JSON.stringify({
          ...failed,
          output: { secret: true },
        })
      )
    ).toThrow('invalid execution progress');
  });

  test('the reviewed WOML source deep-equals the frozen Model v6 fixture', async () => {
    const source = await Bun.file(
      join(womlFixtureDirectory, 'retry.woml')
    ).text();
    const expected = (await readJson(
      join(womlFixtureDirectory, 'retry.compiled.v6.json')
    )) as CompiledWorkflowDefinition;
    const compiled = compileWoml(parseWoml(source, { file: 'retry.woml' }));
    const { model } = await retryValidators();

    expect(compiled).toEqual(expected);
    expect(model(compiled), validationMessage(model)).toBe(true);
  });

  test('Model v6 rejects retry on structural nodes and malformed policies', async () => {
    const { model } = await retryValidators();
    const compiled = (await readJson(
      join(womlFixtureDirectory, 'retry.compiled.v6.json')
    )) as JsonObject;
    const graph = compiled.graph as JsonObject;
    const nodes = graph.nodes as JsonObject[];

    const structural = structuredClone(compiled);
    const structuralGraph = structural.graph as JsonObject;
    (structuralGraph.nodes as JsonObject[])[1] = {
      id: 'join',
      handler: 'engine.parallel-join',
      inputs: { kind: 'object', fields: {} },
      retryPolicy: nodes[1].retryPolicy,
    };
    expect(model(structural)).toBe(false);

    const malformed = structuredClone(compiled);
    const malformedPolicy = (
      ((malformed.graph as JsonObject).nodes as JsonObject[])[1]
        .retryPolicy as JsonObject
    ).backoff as JsonObject;
    malformedPolicy.multiplier = 3;
    expect(model(malformed)).toBe(false);
  });

  test('all four reviewed Event v6 histories validate structurally and semantically', async () => {
    const { event } = await retryValidators();
    const definition = await readJson(
      join(womlFixtureDirectory, 'retry.compiled.v6.json')
    );
    const expectedDefinitionHash = sha256Canonical(definition);
    const names = readdirSync(eventFixtureDirectory)
      .filter(name => name.endsWith('.events.v6.json'))
      .sort();
    expect(names).toEqual([
      'retry-ambiguous-recovery.events.v6.json',
      'retry-exhausted.events.v6.json',
      'retry-scheduled-recovery.events.v6.json',
      'retry-success.events.v6.json',
    ]);

    for (const name of names) {
      const history = (await readJson(
        join(eventFixtureDirectory, name)
      )) as unknown[];
      for (const item of history) {
        expect(event(item), `${name}: ${validationMessage(event)}`).toBe(true);
      }
      expect(historyIssues(history), name).toEqual([]);

      const started = history.filter(
        item => (item as JsonObject).type === 'step_attempt_started'
      ) as JsonObject[];
      const runStarted = history[0] as JsonObject;
      const runStartedData = runStarted.data as JsonObject;
      expect(runStartedData.definitionHash).toBe(expectedDefinitionHash);
      for (const item of started) {
        const data = item.data as JsonObject;
        expect(data.idempotencyKey).toBe(
          sha256Canonical({
            contract: 'woml.step-effect',
            version: 1,
            runId: item.runId,
            definitionHash: expectedDefinitionHash,
            nodeId: data.nodeId,
          })
        );
      }
    }
  });

  test('Event v6 requires a stable key and semantic checks reject unsafe schedules', async () => {
    const { event } = await retryValidators();
    const history = (await readJson(
      join(eventFixtureDirectory, 'retry-success.events.v6.json')
    )) as JsonObject[];

    const missingKey = structuredClone(history[1]);
    delete (missingKey.data as JsonObject).idempotencyKey;
    expect(event(missingKey)).toBe(false);

    const wrongNextAttempt = structuredClone(history);
    const schedule = wrongNextAttempt.find(
      item => item.type === 'step_retry_scheduled'
    )!;
    (schedule.data as JsonObject).nextAttempt = 3;
    expect(historyIssues(wrongNextAttempt)).toContain(
      'nextAttempt must equal failedAttempt + 1.'
    );

    const changedKey = structuredClone(history);
    const secondStart = changedKey.find(
      item =>
        item.type === 'step_attempt_started' &&
        (item.data as JsonObject).attempt === 2
    )!;
    (secondStart.data as JsonObject).idempotencyKey =
      'sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc';
    expect(historyIssues(changedKey)).toContain(
      'greet changes its idempotency key.'
    );
  });

  test('all Script Host v3 fixtures validate and framing counts UTF-8 bytes', async () => {
    const { host } = await retryValidators();
    const names = readdirSync(hostFixtureDirectory).sort();
    for (const name of names) {
      const fixture = await readJson(join(hostFixtureDirectory, name));
      expect(host(fixture), `${name}: ${validationMessage(host)}`).toBe(true);
    }

    const execute = (await readJson(
      join(hostFixtureDirectory, 'execute.v3.json')
    )) as JsonObject;
    const source = String(execute.source);
    expect(source).toContain('Héllo');
    expect(source).toContain('\r\n');
    const frame = encodeFrame(execute);
    const headerEnd = frame.indexOf('\r\n\r\n');
    const declared = Number(
      /^Content-Length: ([0-9]+)$/.exec(
        frame.subarray(0, headerEnd).toString('ascii')
      )![1]
    );
    expect(declared).toBe(frame.subarray(headerEnd + 4).byteLength);
    expect(declared).toBeGreaterThan(JSON.stringify(execute).length);
  });

  test('all progress fixtures validate and secret-bearing fields are rejected', async () => {
    const { progress } = await retryValidators();
    const names = readdirSync(progressFixtureDirectory).sort();
    for (const name of names) {
      const fixture = await readJson(join(progressFixtureDirectory, name));
      expect(
        progress(fixture),
        `${basename(name)}: ${validationMessage(progress)}`
      ).toBe(true);
    }

    const leaked = (await readJson(
      join(progressFixtureDirectory, 'attempt-failed.v1.json')
    )) as JsonObject;
    leaked.context = { secrets: { TOKEN: 'forbidden' } };
    expect(progress(leaked)).toBe(false);
  });
});
