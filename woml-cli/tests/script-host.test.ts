import { describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';

import {
  encodeFrame,
  FrameDecoder,
  FrameProtocolError,
} from '../src/script-host/framing';
import { ScriptHost } from '../src/script-host/host';
import { isScriptHostMessage } from '../src/script-host/protocol';
import type {
  CapabilityCallMessage,
  CapabilityResultMessage,
  CancelMessage,
  CompletedMessage,
  ExecuteMessage,
  ExecuteMessageV4,
  ExecuteMessageV6,
  ExecuteMessageV7,
  FetchObservationMessage,
  ScriptAttempt,
  ScriptHostMessage,
} from '../src/script-host/types';

const packageRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(packageRoot, '..');
const hostEntry =
  process.env.WOML_SCRIPT_HOST_TEST_ENTRY ??
  resolve(packageRoot, 'src/script-host.ts');
const bunExecutable = Bun.which('bun')!;
const defaultEffectKey =
  'sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef';

function execute(
  invocationId: string,
  source: string,
  options: {
    readonly context?: ExecuteMessage['context'];
    readonly timeoutMs?: number;
    readonly protocolVersion?: 1 | 2 | 3;
    readonly attempt?: ScriptAttempt;
  } = {}
): ExecuteMessage {
  const protocolVersion = options.protocolVersion ?? 3;
  const base = {
    protocol: 'woml.script-host',
    messageType: 'execute',
    invocationId,
    runId: 'run_host_test_01',
    nodeId: invocationId.replace(/^inv_/, ''),
    handler: 'runtime.script',
    timeoutMs: options.timeoutMs ?? 1000,
    source,
    context: options.context ?? { trigger: {}, steps: {} },
  } as const;
  return protocolVersion === 3
    ? {
        ...base,
        protocolVersion,
        attempt: options.attempt ?? {
          number: 1,
          maxAttempts: 1,
          idempotencyKey: defaultEffectKey,
        },
      }
    : { ...base, protocolVersion, attempt: 1 };
}

function executeV4(
  invocationId: string,
  source: string,
  secrets: Readonly<Record<string, string>> = {}
): ExecuteMessage {
  return {
    protocol: 'woml.script-host',
    protocolVersion: 4,
    messageType: 'execute',
    invocationId,
    runId: `run_${invocationId}`,
    nodeId: 'serviceStep',
    attempt: {
      number: 1,
      maxAttempts: 1,
      idempotencyKey: defaultEffectKey,
    },
    handler: 'runtime.script',
    timeoutMs: 2_000,
    source,
    context: { trigger: {}, steps: {} },
    bindings: { bindingVersion: 1, servicesVersion: 1, secrets },
  };
}

function moduleDigest(bundle: string): string {
  return `sha256:${new Bun.CryptoHasher('sha256')
    .update(bundle)
    .digest('hex')}`;
}

function executeV5(
  invocationId: string,
  source: string,
  bundleDigest: string,
  exports: readonly string[]
): ExecuteMessage {
  const base = executeV4(invocationId, source) as ExecuteMessageV4;
  return {
    ...base,
    protocolVersion: 5,
    modules: [{ name: 'utility', bundleDigest, exports }],
  };
}

function executeV6(
  invocationId: string,
  source: string,
  bundleDigest: string,
  exports: readonly string[],
  secrets: Readonly<Record<string, string>> = {}
): ExecuteMessageV6 {
  const base = executeV4(invocationId, source, secrets) as ExecuteMessageV4;
  return {
    ...base,
    protocolVersion: 6,
    modules: [{ name: 'utility', bundleDigest, exports }],
  };
}

function executeLifecycleV7(
  invocationId: string,
  source: string
): ExecuteMessageV7 {
  return {
    protocol: 'woml.script-host',
    protocolVersion: 7,
    messageType: 'execute',
    invocationId,
    runId: `run_${invocationId}`,
    nodeId: 'lifecycle:run_start:action:0',
    attempt: {
      number: 1,
      maxAttempts: 1,
      idempotencyKey: defaultEffectKey,
    },
    mode: 'lifecycle',
    handler: 'runtime.lifecycle-script',
    timeoutMs: 2_000,
    source,
    context: { trigger: { orderId: 'order-1' }, steps: {} },
    lifecycle: {
      event: 'run_start',
      workflow: { id: 'lifecycle-test' },
    },
    bindings: { bindingVersion: 1, servicesVersion: 1, secrets: {} },
    modules: [],
  };
}

function executeStepV7(invocationId: string, source: string): ExecuteMessageV7 {
  const base = executeV4(invocationId, source) as ExecuteMessageV4;
  return {
    ...base,
    protocolVersion: 7,
    mode: 'step',
    handler: 'runtime.script',
    modules: [],
  };
}

function cancel(
  invocationId: string,
  protocolVersion: 2 | 3 = 3
): CancelMessage {
  return {
    protocol: 'woml.script-host',
    protocolVersion,
    messageType: 'cancel',
    invocationId,
    reason: 'parallel_fail_fast',
  };
}

interface HostRunResult {
  readonly messages: ScriptHostMessage[];
  readonly stderr: string;
  readonly exitCode: number;
}

async function runHost(
  messages: readonly unknown[],
  environment: Readonly<Record<string, string>> = {}
): Promise<HostRunResult> {
  const input = Buffer.concat(messages.map(encodeFrame));
  const child = Bun.spawn([bunExecutable, hostEntry], {
    cwd: projectRoot,
    env: { ...process.env, ...environment },
    stdin: input,
    stdout: 'pipe',
    stderr: 'pipe',
  });
  const stdoutPromise = new Response(child.stdout).arrayBuffer();
  const stderrPromise = new Response(child.stderr).text();
  const watchdog = setTimeout(() => child.kill(), 5000);

  const [stdout, stderr, exitCode] = await Promise.all([
    stdoutPromise,
    stderrPromise,
    child.exited,
  ]);
  clearTimeout(watchdog);
  const decoder = new FrameDecoder();
  const decoded = decoder.push(new Uint8Array(stdout));
  decoder.finish();
  for (const message of decoded)
    expect(isScriptHostMessage(message)).toBe(true);

  return {
    messages: decoded as ScriptHostMessage[],
    stderr,
    exitCode,
  };
}

function completions(
  messages: readonly ScriptHostMessage[]
): CompletedMessage[] {
  return messages.filter(
    (message): message is CompletedMessage =>
      message.messageType === 'completed'
  );
}

function byInvocation(
  messages: readonly ScriptHostMessage[]
): ReadonlyMap<string, CompletedMessage> {
  return new Map(
    completions(messages).map(message => [message.invocationId, message])
  );
}

describe('production Content-Length framing', () => {
  test('decodes multibyte and CRLF content one byte at a time', async () => {
    const fixture = await Bun.file(
      resolve(
        packageRoot,
        'tests/fixtures/script-host/unicode-crlf.execute.v1.json'
      )
    ).json();
    const frame = encodeFrame(fixture);
    const body = JSON.stringify(fixture);
    expect(new TextEncoder().encode(body).byteLength).toBeGreaterThan(
      body.length
    );

    const decoder = new FrameDecoder();
    const messages: unknown[] = [];
    for (const byte of frame)
      messages.push(...decoder.push(Uint8Array.of(byte)));
    decoder.finish();

    expect(messages).toEqual([fixture]);
  });

  test('decodes combined frames and rejects malformed, oversized, or incomplete input', () => {
    const first = { id: 1 };
    const second = { id: 2 };
    const decoder = new FrameDecoder();
    expect(
      decoder.push(Buffer.concat([encodeFrame(first), encodeFrame(second)]))
    ).toEqual([first, second]);
    decoder.finish();

    expect(() =>
      new FrameDecoder().push(
        Buffer.from('Content-Length: nope\r\n\r\n', 'ascii')
      )
    ).toThrow(FrameProtocolError);
    expect(() =>
      new FrameDecoder({ maxFrameBytes: 10 }).push(
        Buffer.from('Content-Length: 11\r\n\r\n', 'ascii')
      )
    ).toThrow(FrameProtocolError);
    const incomplete = new FrameDecoder();
    incomplete.push(Buffer.from('Content-Length: 4\r\n\r\n{}', 'ascii'));
    expect(() => incomplete.finish()).toThrow(FrameProtocolError);
    const invalidUtf8Body = Buffer.from([0x22, 0xc3, 0x28, 0x22]);
    expect(() =>
      new FrameDecoder().push(
        Buffer.concat([
          Buffer.from('Content-Length: 4\r\n\r\n', 'ascii'),
          invalidUtf8Body,
        ])
      )
    ).toThrow(FrameProtocolError);
  });
});

describe('long-lived Bun script host', () => {
  test('preserves protocol v1 execution when explicitly requested', async () => {
    const result = await runHost(
      [
        execute('inv_legacy_v1', 'return { legacy: true };', {
          protocolVersion: 1,
        }),
      ],
      { WOML_SCRIPT_HOST_PROTOCOL_VERSION: '1' }
    );
    expect(result.exitCode).toBe(0);
    expect(result.messages[0]).toMatchObject({
      messageType: 'ready',
      protocolVersion: 1,
    });
    expect(byInvocation(result.messages).get('inv_legacy_v1')).toMatchObject({
      protocolVersion: 1,
      outcome: { kind: 'success', value: { legacy: true } },
    });
  });

  test('preserves protocol v2 execution when explicitly requested', async () => {
    const result = await runHost(
      [
        execute('inv_legacy_v2', 'return { legacy: 2 };', {
          protocolVersion: 2,
        }),
      ],
      { WOML_SCRIPT_HOST_PROTOCOL_VERSION: '2' }
    );
    expect(result.exitCode).toBe(0);
    expect(result.messages[0]).toMatchObject({
      messageType: 'ready',
      protocolVersion: 2,
    });
    expect(byInvocation(result.messages).get('inv_legacy_v2')).toMatchObject({
      protocolVersion: 2,
      outcome: { kind: 'success', value: { legacy: 2 } },
    });
  });

  test('provides the frozen context contract with top-level await', async () => {
    const result = await runHost([
      execute(
        'inv_context_contract',
        `await Promise.resolve();
return {
  greeting: \`Hello \${context.payload.name}\`,
  legacyAlias: context.trigger.name,
  hasRun: "run" in context,
  hasServices: typeof services !== "undefined"
};`,
        { context: { trigger: { name: 'Ada' }, steps: {} } }
      ),
      execute(
        'inv_frozen_context',
        'context.payload.name = "Changed"; return { unreachable: true };',
        { context: { trigger: { name: 'Original' }, steps: {} } }
      ),
    ]);
    const indexed = byInvocation(result.messages);

    expect(indexed.get('inv_context_contract')?.outcome).toEqual({
      kind: 'success',
      value: {
        greeting: 'Hello Ada',
        legacyAlias: 'Ada',
        hasRun: false,
        hasServices: false,
      },
    });
    expect(indexed.get('inv_frozen_context')?.outcome).toMatchObject({
      kind: 'failure',
      error: { kind: 'script_threw', code: 'WOML_SCRIPT_THROWN' },
    });
  });

  test('provides only the deeply frozen Protocol v3 attempt binding', async () => {
    const stableKey =
      'sha256:35278a8c79c5843d1fc3015aac65ea3ee7579559463214234e16624b5bbf609c';
    const source = `
let mutation = 'not-blocked';
try { attempt.number = 99; } catch { mutation = 'blocked'; }
return {
  number: attempt.number,
  maxAttempts: attempt.maxAttempts,
  idempotencyKey: attempt.idempotencyKey,
  frozen: Object.isFrozen(attempt),
  mutation,
  contextContainsAttempt: 'attempt' in context,
  hasRunId: 'runId' in attempt,
  hasNodeId: 'nodeId' in attempt,
  hasInvocationId: 'invocationId' in attempt,
  env: process.env.WOML_TEST_SECRET ?? null
};`;
    const result = await runHost(
      [
        execute('inv_attempt_01', source, {
          attempt: {
            number: 1,
            maxAttempts: 3,
            idempotencyKey: stableKey,
          },
        }),
        execute('inv_attempt_02', source, {
          attempt: {
            number: 2,
            maxAttempts: 3,
            idempotencyKey: stableKey,
          },
        }),
      ],
      { WOML_TEST_SECRET: 'must-not-enter-attempt' }
    );
    const indexed = byInvocation(result.messages);

    expect(indexed.get('inv_attempt_01')?.outcome).toEqual({
      kind: 'success',
      value: {
        number: 1,
        maxAttempts: 3,
        idempotencyKey: stableKey,
        frozen: true,
        mutation: 'blocked',
        contextContainsAttempt: false,
        hasRunId: false,
        hasNodeId: false,
        hasInvocationId: false,
        env: null,
      },
    });
    expect(indexed.get('inv_attempt_02')?.outcome).toMatchObject({
      kind: 'success',
      value: {
        number: 2,
        maxAttempts: 3,
        idempotencyKey: stableKey,
      },
    });
  });

  test('multiplexes invocations and correlates out-of-order responses', async () => {
    const result = await runHost([
      execute(
        'inv_slow',
        'await new Promise((resolve) => setTimeout(resolve, 180)); return { order: "slow" };'
      ),
      execute(
        'inv_fast',
        'await new Promise((resolve) => setTimeout(resolve, 5)); return { order: "fast" };'
      ),
      execute(
        'inv_middle',
        'await new Promise((resolve) => setTimeout(resolve, 80)); return { order: "middle" };'
      ),
    ]);

    expect(result.exitCode).toBe(0);
    expect(result.stderr).toBe('');
    expect(result.messages[0].messageType).toBe('ready');
    expect(
      completions(result.messages).map(message => message.invocationId)
    ).toEqual(['inv_fast', 'inv_middle', 'inv_slow']);
    const indexed = byInvocation(result.messages);
    expect(indexed.get('inv_fast')?.outcome).toEqual({
      kind: 'success',
      value: { order: 'fast' },
    });
    expect(indexed.get('inv_slow')?.outcome).toEqual({
      kind: 'success',
      value: { order: 'slow' },
    });
  });

  test('cancels only the addressed Worker and emits one terminal response', async () => {
    const result = await runHost([
      execute(
        'inv_cancel_me',
        'await new Promise(resolve => setTimeout(resolve, 500)); return { tooLate: true };'
      ),
      execute('inv_unrelated', 'return { survived: true };'),
      cancel('inv_cancel_me'),
      cancel('inv_unknown'),
    ]);
    const indexed = byInvocation(result.messages);

    expect(result.exitCode).toBe(0);
    expect(indexed.get('inv_cancel_me')?.outcome).toEqual({
      kind: 'failure',
      error: {
        kind: 'invocation_cancelled',
        code: 'WOML_SCRIPT_CANCELLED',
        message: 'Invocation was cancelled by parallel fail-fast.',
      },
    });
    expect(indexed.get('inv_unrelated')?.outcome).toEqual({
      kind: 'success',
      value: { survived: true },
    });
    expect(
      completions(result.messages).filter(
        message => message.invocationId === 'inv_cancel_me'
      )
    ).toHaveLength(1);
  });

  test('a late cancel is a safe no-op after the real completion wins', async () => {
    const sent: CompletedMessage[] = [];
    const host = new ScriptHost({
      workerUrl: new URL('../src/script-host-worker.ts', import.meta.url),
      protocolVersion: 3,
      send: async message => {
        if (message.messageType === 'completed') sent.push(message);
      },
    });
    host.accept(execute('inv_completed_first', 'return { won: true };'));
    await host.drain();
    host.accept(cancel('inv_completed_first'));
    host.accept(cancel('inv_never_existed'));
    await host.drain();

    expect(sent).toHaveLength(1);
    expect(sent[0].outcome).toEqual({
      kind: 'success',
      value: { won: true },
    });
  });

  test('uses fresh Worker globals and does not pass host environment secrets', async () => {
    const result = await runHost(
      [
        execute(
          'inv_set_global',
          'globalThis.__womlLeak = "secret-state"; return { set: true };'
        ),
        execute(
          'inv_read_global',
          'await new Promise((resolve) => setTimeout(resolve, 40)); return { leaked: globalThis.__womlLeak ?? null, env: process.env.WOML_TEST_SECRET ?? null };'
        ),
      ],
      { WOML_TEST_SECRET: 'must-not-enter-worker' }
    );

    expect(result.exitCode).toBe(0);
    expect(
      byInvocation(result.messages).get('inv_read_global')?.outcome
    ).toEqual({
      kind: 'success',
      value: { leaked: null, env: null },
    });
  });

  test('keeps throw, invalid result, and timeout distinct while the host stays alive', async () => {
    const result = await runHost([
      execute('inv_throw', 'throw new Error("boom");'),
      execute('inv_non_json', 'return 1n;'),
      execute('inv_timeout', 'while (true) {}', { timeoutMs: 40 }),
      execute('inv_after_timeout', 'return { hostAlive: true };'),
    ]);
    const indexed = byInvocation(result.messages);

    expect(result.exitCode).toBe(0);
    expect(indexed.get('inv_throw')?.outcome).toMatchObject({
      kind: 'failure',
      error: { kind: 'script_threw', code: 'WOML_SCRIPT_THROWN' },
    });
    expect(indexed.get('inv_non_json')?.outcome).toMatchObject({
      kind: 'failure',
      error: {
        kind: 'invalid_script_result',
        code: 'WOML_SCRIPT_NON_JSON_RESULT',
      },
    });
    expect(indexed.get('inv_timeout')?.outcome).toMatchObject({
      kind: 'failure',
      error: { kind: 'script_timed_out', code: 'WOML_SCRIPT_TIMEOUT' },
    });
    expect(indexed.get('inv_after_timeout')?.outcome).toEqual({
      kind: 'success',
      value: { hostAlive: true },
    });
  });

  test('rejects every non-JSON result shape covered by the retired executor', async () => {
    const result = await runHost([
      execute('inv_undefined', 'return undefined;'),
      execute('inv_bigint', 'return 1n;'),
      execute('inv_function', 'return { callback() {} };'),
      execute(
        'inv_circular',
        'const value = {}; value.self = value; return value;'
      ),
    ]);
    const indexed = byInvocation(result.messages);

    for (const invocationId of [
      'inv_undefined',
      'inv_bigint',
      'inv_function',
      'inv_circular',
    ]) {
      expect(indexed.get(invocationId)?.outcome).toMatchObject({
        kind: 'failure',
        error: {
          kind: 'invalid_script_result',
          code: 'WOML_SCRIPT_NON_JSON_RESULT',
        },
      });
    }
  });

  test('a Worker crash racing with cancel produces exactly one terminal outcome', async () => {
    const sent: CompletedMessage[] = [];
    const host = new ScriptHost({
      workerUrl: new URL('./fixtures/missing-worker.ts', import.meta.url),
      send: async message => {
        if (message.messageType === 'completed') sent.push(message);
      },
    });

    host.accept(execute('inv_worker_crash', 'return { unreachable: true };'));
    host.accept(cancel('inv_worker_crash'));
    await host.drain();

    expect(sent).toHaveLength(1);
    expect(sent[0].outcome.kind).toBe('failure');
    if (sent[0].outcome.kind !== 'failure') {
      throw new Error('Expected one failure outcome.');
    }
    expect([
      ['worker_crashed', 'WOML_SCRIPT_WORKER_CRASHED'],
      ['invocation_cancelled', 'WOML_SCRIPT_CANCELLED'],
    ]).toContainEqual([sent[0].outcome.error.kind, sent[0].outcome.error.code]);
  });

  test('enforces context and result byte limits with separate failures', async () => {
    const result = await runHost(
      [
        execute('inv_context_large', 'return { unreachable: true };', {
          context: {
            trigger: { payload: 'x'.repeat(300) },
            steps: {},
          },
        }),
        execute('inv_result_large', 'return { payload: "x".repeat(300) };'),
      ],
      {
        WOML_SCRIPT_HOST_MAX_CONTEXT_BYTES: '200',
        WOML_SCRIPT_HOST_MAX_RESULT_BYTES: '200',
      }
    );
    const indexed = byInvocation(result.messages);

    expect(indexed.get('inv_context_large')?.outcome).toMatchObject({
      kind: 'failure',
      error: {
        kind: 'context_too_large',
        code: 'WOML_SCRIPT_CONTEXT_TOO_LARGE',
        details: { limitBytes: 200 },
      },
    });
    expect(indexed.get('inv_result_large')?.outcome).toMatchObject({
      kind: 'failure',
      error: {
        kind: 'result_too_large',
        code: 'WOML_SCRIPT_RESULT_TOO_LARGE',
        details: { limitBytes: 200 },
      },
    });
  });

  test('fails closed on a schema-invalid Rust message', async () => {
    const invalid = {
      ...execute('inv_invalid', 'return { ok: true };'),
      attempt: {
        number: 2,
        maxAttempts: 1,
        idempotencyKey: defaultEffectKey,
      },
    };
    const result = await runHost([invalid]);

    expect(result.exitCode).toBe(1);
    expect(result.messages).toHaveLength(1);
    expect(result.messages[0].messageType).toBe('ready');
    expect(result.stderr).toContain('MessageProtocolError');
    expect(result.stderr).not.toContain('return { ok: true }');
  });

  test('protocol v4 routes Promise.all capability replies out of order and isolates simultaneous runs', async () => {
    const completed: CompletedMessage[] = [];
    const calls: CapabilityCallMessage[] = [];
    let host!: ScriptHost;
    host = new ScriptHost({
      workerUrl: new URL('../src/script-host-worker.ts', import.meta.url),
      protocolVersion: 4,
      send: async message => {
        if (message.messageType === 'completed') {
          completed.push(message);
          return;
        }
        if (message.messageType !== 'capability_call') return;
        calls.push(message);
        const input = message.call.input as {
          readonly value: string;
          readonly delayMs: number;
        };
        setTimeout(() => {
          const result: CapabilityResultMessage = {
            protocol: 'woml.script-host',
            protocolVersion: 4,
            messageType: 'capability_result',
            invocationId: message.invocationId,
            callId: message.callId,
            result: {
              contract: 'woml.capability-call',
              contractVersion: 1,
              messageType: 'result',
              invocationId: message.invocationId,
              callId: message.callId,
              outcome: 'succeeded',
              resultContractVersion: 1,
              resultBytes: Buffer.byteLength(JSON.stringify(input.value)),
              durationMs: input.delayMs,
              result: input.value,
            },
          };
          host.accept(result);
        }, input.delayMs);
      },
    });

    host.accept(
      executeV4(
        'inv_v4_a',
        `const [slow, fast] = await Promise.all([
          services.test.control({ value: 'a-slow', delayMs: 40 }),
          services.test.control({ value: 'a-fast', delayMs: 2 })
        ]); return { slow, fast };`
      )
    );
    host.accept(
      executeV4(
        'inv_v4_b',
        `return { value: await services.test.control({ value: 'b', delayMs: 1 }) };`
      )
    );
    await host.drain();

    expect(calls).toHaveLength(3);
    expect(byInvocation(completed).get('inv_v4_a')?.outcome).toEqual({
      kind: 'success',
      value: { slow: 'a-slow', fast: 'a-fast' },
    });
    expect(byInvocation(completed).get('inv_v4_b')?.outcome).toEqual({
      kind: 'success',
      value: { value: 'b' },
    });
  });

  test('protocol v4 lowers managed HTTP defaults and exposes only its public result', async () => {
    const completed: CompletedMessage[] = [];
    const calls: CapabilityCallMessage[] = [];
    let host!: ScriptHost;
    host = new ScriptHost({
      workerUrl: new URL('../src/script-host-worker.ts', import.meta.url),
      protocolVersion: 4,
      send: async message => {
        if (message.messageType === 'completed') {
          completed.push(message);
          return;
        }
        if (message.messageType !== 'capability_call') return;
        calls.push(message);
        const managedResult = {
          contract: 'woml.managed-http',
          contractVersion: 1,
          kind: 'result',
          status: 201,
          ok: true,
          headers: { 'content-type': 'application/json' },
          data: { created: true },
          url: 'https://api.example.test/orders?expand=customer',
          redirected: false,
        } as const;
        host.accept({
          protocol: 'woml.script-host',
          protocolVersion: 4,
          messageType: 'capability_result',
          invocationId: message.invocationId,
          callId: message.callId,
          result: {
            contract: 'woml.capability-call',
            contractVersion: 1,
            messageType: 'result',
            invocationId: message.invocationId,
            callId: message.callId,
            outcome: 'succeeded',
            resultContractVersion: 1,
            resultBytes: Buffer.byteLength(JSON.stringify(managedResult)),
            durationMs: 3,
            result: managedResult,
          },
        });
      },
    });
    host.accept(
      executeV4(
        'inv_v4_managed_http',
        `return await services.http.request({
          url: 'https://api.example.test/orders',
          method: 'post',
          query: { expand: 'customer' },
          json: { amount: 42 },
          timeout: '2s',
          idempotency: { header: 'Idempotency-Key', value: 'order-42' }
        }, { name: 'create-order' });`
      )
    );
    await host.drain();

    expect(calls).toHaveLength(1);
    expect(calls[0]?.call).toMatchObject({
      capability: 'http',
      operation: 'request',
      identity: {
        mode: 'named',
        operationName: 'http.request.create-order',
        providerIdempotencyKey: 'order-42',
      },
      limits: { timeoutMs: 3_000 },
      input: {
        contract: 'woml.managed-http',
        contractVersion: 1,
        kind: 'request',
        method: 'POST',
        headers: {},
        responseType: 'json',
        timeoutMs: 2_000,
        acceptedStatus: { minimum: 200, maximum: 299 },
        redirect: 'follow',
        maximumRedirects: 10,
      },
    });
    expect(completed[0]?.outcome).toEqual({
      kind: 'success',
      value: {
        status: 201,
        ok: true,
        headers: { 'content-type': 'application/json' },
        data: { created: true },
        url: 'https://api.example.test/orders?expand=customer',
        redirected: false,
      },
    });
  });

  test('protocol v4 requires names when a step makes multiple effectful managed HTTP calls', async () => {
    const completed: CompletedMessage[] = [];
    let callCount = 0;
    let host!: ScriptHost;
    host = new ScriptHost({
      workerUrl: new URL('../src/script-host-worker.ts', import.meta.url),
      protocolVersion: 4,
      send: async message => {
        if (message.messageType === 'completed') {
          completed.push(message);
          return;
        }
        if (message.messageType !== 'capability_call') return;
        callCount += 1;
        const managedResult = {
          contract: 'woml.managed-http',
          contractVersion: 1,
          kind: 'result',
          status: 204,
          ok: true,
          headers: {},
          data: null,
          url: 'https://api.example.test/orders',
          redirected: false,
        } as const;
        host.accept({
          protocol: 'woml.script-host',
          protocolVersion: 4,
          messageType: 'capability_result',
          invocationId: message.invocationId,
          callId: message.callId,
          result: {
            contract: 'woml.capability-call',
            contractVersion: 1,
            messageType: 'result',
            invocationId: message.invocationId,
            callId: message.callId,
            outcome: 'succeeded',
            resultContractVersion: 1,
            resultBytes: Buffer.byteLength(JSON.stringify(managedResult)),
            durationMs: 1,
            result: managedResult,
          },
        });
      },
    });
    host.accept(
      executeV4(
        'inv_v4_effect_names',
        `await services.http.request({
          url: 'https://api.example.test/orders', method: 'POST', responseType: 'text'
        });
        try {
          await services.http.request({
            url: 'https://api.example.test/audit', method: 'POST', responseType: 'text'
          });
          return { rejected: false };
        } catch (error) {
          return { rejected: true, message: error.message };
        }`
      )
    );
    await host.drain();

    expect(callCount).toBe(1);
    expect(completed[0]?.outcome).toMatchObject({
      kind: 'success',
      value: {
        rejected: true,
        message: expect.stringContaining('require stable names'),
      },
    });
  });

  test('WC3/WC4 lowers workflows.call, bounds waiting, and enforces stable repeated identities', async () => {
    const completed: CompletedMessage[] = [];
    const calls: CapabilityCallMessage[] = [];
    let host!: ScriptHost;
    host = new ScriptHost({
      workerUrl: new URL('../src/script-host-worker.ts', import.meta.url),
      protocolVersion: 4,
      send: async message => {
        if (message.messageType === 'completed') {
          completed.push(message);
          return;
        }
        if (message.messageType !== 'capability_call') return;
        calls.push(message);
        const request = message.call.input as {
          readonly workflowId: string;
          readonly payload: { readonly customerId: string };
        };
        const workflowResult =
          message.call.operation === 'start'
            ? ({
                contract: 'woml.workflow-start',
                contractVersion: 1,
                kind: 'started',
                workflowId: request.workflowId,
                runId: 'run_start_test',
                duplicate: false,
              } as const)
            : ({
                contract: 'woml.workflow-call',
                contractVersion: 1,
                kind: 'succeeded',
                workflowId: request.workflowId,
                definitionHash:
                  'sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
                childRunId: 'run_call_test',
                result: {
                  score: request.payload.customerId === 'customer-42' ? 90 : 20,
                },
              } as const);
        host.accept({
          protocol: 'woml.script-host',
          protocolVersion: 4,
          messageType: 'capability_result',
          invocationId: message.invocationId,
          callId: message.callId,
          result: {
            contract: 'woml.capability-call',
            contractVersion: 1,
            messageType: 'result',
            invocationId: message.invocationId,
            callId: message.callId,
            outcome: 'succeeded',
            resultContractVersion: 1,
            resultBytes: Buffer.byteLength(JSON.stringify(workflowResult)),
            durationMs: 1,
            result: workflowResult,
          },
        });
      },
    });
    host.accept(
      executeV4(
        'inv_v4_workflow_start',
        `const started = await services.workflows.start(
          'calculate-risk',
          { customerId: 'customer-42' },
          { name: 'background-risk' }
        );
        return { started, frozen: Object.isFrozen(started) };`
      )
    );
    host.accept(
      executeV4(
        'inv_v4_workflow_call',
        `const risk = await services.workflows.call(
          'calculate-risk',
          { customerId: 'customer-42' },
          { name: 'customer-risk', timeout: '1s' }
        );
        return { risk, frozen: Object.isFrozen(services.workflows) };`
      )
    );
    host.accept(
      executeV4(
        'inv_v4_workflow_call_timeout_limit',
        `try {
          await services.workflows.call('calculate-risk', {}, { timeout: '3s' });
          return { rejected: false };
        } catch (error) {
          return { rejected: true, message: error.message };
        }`
      )
    );
    host.accept(
      executeV4(
        'inv_v4_workflow_call_automatic_identity',
        `await services.workflows.call('calculate-risk', { customerId: 'customer-42' });
        try {
          await services.workflows.call('calculate-risk', { customerId: 'customer-7' });
          return { rejected: false };
        } catch (error) {
          return { rejected: true, message: error.message };
        }`
      )
    );
    await host.drain();

    expect(calls).toHaveLength(3);
    const mainCall = calls.find(
      call => call.invocationId === 'inv_v4_workflow_call'
    );
    expect(mainCall?.call).toMatchObject({
      capability: 'workflows',
      operation: 'call',
      identity: {
        mode: 'named',
        operationName: 'workflows.call.customer-risk',
      },
      limits: { timeoutMs: 2_000 },
      input: {
        contract: 'woml.workflow-call',
        contractVersion: 1,
        kind: 'request',
        workflowId: 'calculate-risk',
        payload: { customerId: 'customer-42' },
        options: { name: 'customer-risk', timeoutMs: 1_000 },
      },
    });
    const startCall = calls.find(
      call => call.invocationId === 'inv_v4_workflow_start'
    );
    expect(startCall?.call).toMatchObject({
      capability: 'workflows',
      operation: 'start',
      identity: {
        mode: 'named',
        operationName: 'workflows.start.background-risk',
      },
      input: {
        contract: 'woml.workflow-start',
        contractVersion: 1,
        kind: 'request',
        workflowId: 'calculate-risk',
        payload: { customerId: 'customer-42' },
        options: { name: 'background-risk' },
      },
    });
    expect(
      byInvocation(completed).get('inv_v4_workflow_call')?.outcome
    ).toEqual({
      kind: 'success',
      value: { risk: { score: 90 }, frozen: true },
    });
    expect(
      byInvocation(completed).get('inv_v4_workflow_start')?.outcome
    ).toEqual({
      kind: 'success',
      value: {
        started: {
          workflowId: 'calculate-risk',
          runId: 'run_start_test',
          duplicate: false,
        },
        frozen: true,
      },
    });
    expect(
      byInvocation(completed).get('inv_v4_workflow_call_timeout_limit')?.outcome
    ).toEqual({
      kind: 'success',
      value: {
        rejected: true,
        message:
          'Workflow Call timeout cannot exceed the calling step remaining timeout.',
      },
    });
    expect(
      byInvocation(completed).get('inv_v4_workflow_call_automatic_identity')
        ?.outcome
    ).toEqual({
      kind: 'success',
      value: {
        rejected: true,
        message: expect.stringContaining('require stable names'),
      },
    });
  });

  test('protocol v4 lowers services.db() calls and exposes only Database v1 data', async () => {
    const completed: CompletedMessage[] = [];
    const calls: CapabilityCallMessage[] = [];
    let host!: ScriptHost;
    host = new ScriptHost({
      workerUrl: new URL('../src/script-host-worker.ts', import.meta.url),
      protocolVersion: 4,
      send: async message => {
        if (message.messageType === 'completed') {
          completed.push(message);
          return;
        }
        if (message.messageType !== 'capability_call') return;
        calls.push(message);
        const databaseResult = {
          contract: 'woml.database',
          contractVersion: 1,
          kind: 'result',
          operation: message.call.operation,
          data:
            message.call.operation === 'insert'
              ? { rowsAffected: 1, lastInsertId: 7 }
              : { rows: [{ id: 7, name: 'Ada' }], rowCount: 1 },
        } as const;
        host.accept({
          protocol: 'woml.script-host',
          protocolVersion: 4,
          messageType: 'capability_result',
          invocationId: message.invocationId,
          callId: message.callId,
          result: {
            contract: 'woml.capability-call',
            contractVersion: 1,
            messageType: 'result',
            invocationId: message.invocationId,
            callId: message.callId,
            outcome: 'succeeded',
            resultContractVersion: 1,
            resultBytes: Buffer.byteLength(JSON.stringify(databaseResult)),
            durationMs: 1,
            result: databaseResult,
          },
        });
      },
    });
    host.accept(
      executeV4(
        'inv_v4_database',
        `const db = services.db({ driver: 'sqlite', connection: './customers.sqlite' });
        const inserted = await db.insert({
          table: 'customers', values: { name: 'Ada' }
        }, { name: 'create-customer' });
        const selected = await db.query({
          text: 'SELECT id, name FROM customers WHERE id = ?', values: [inserted.lastInsertId]
        });
        return { inserted, selected, frozen: Object.isFrozen(db) };`
      )
    );
    await host.drain();

    expect(calls).toHaveLength(2);
    expect(calls[0]?.call).toMatchObject({
      capability: 'db',
      operation: 'insert',
      identity: {
        mode: 'named',
        operationName: 'db.insert.create-customer',
      },
      input: {
        contract: 'woml.database',
        contractVersion: 1,
        kind: 'request',
        driver: 'sqlite',
        connection: './customers.sqlite',
        operation: 'insert',
        input: { table: 'customers', values: { name: 'Ada' } },
      },
    });
    expect(calls[1]?.call).toMatchObject({
      capability: 'db',
      operation: 'query',
      identity: { mode: 'automatic', operationName: 'db.query' },
    });
    expect(completed[0]?.outcome).toEqual({
      kind: 'success',
      value: {
        inserted: { rowsAffected: 1, lastInsertId: 7 },
        selected: { rows: [{ id: 7, name: 'Ada' }], rowCount: 1 },
        frozen: true,
      },
    });
  });

  test('protocol v4 forwards PostgreSQL through the same Database v1 facade', async () => {
    const completed: CompletedMessage[] = [];
    const calls: CapabilityCallMessage[] = [];
    let host!: ScriptHost;
    host = new ScriptHost({
      workerUrl: new URL('../src/script-host-worker.ts', import.meta.url),
      protocolVersion: 4,
      send: async message => {
        if (message.messageType === 'completed') {
          completed.push(message);
          return;
        }
        if (message.messageType !== 'capability_call') return;
        calls.push(message);
        const databaseResult = {
          contract: 'woml.database',
          contractVersion: 1,
          kind: 'result',
          operation: 'query',
          data: { rows: [{ ready: true }], rowCount: 1 },
        } as const;
        host.accept({
          protocol: 'woml.script-host',
          protocolVersion: 4,
          messageType: 'capability_result',
          invocationId: message.invocationId,
          callId: message.callId,
          result: {
            contract: 'woml.capability-call',
            contractVersion: 1,
            messageType: 'result',
            invocationId: message.invocationId,
            callId: message.callId,
            outcome: 'succeeded',
            resultContractVersion: 1,
            resultBytes: Buffer.byteLength(JSON.stringify(databaseResult)),
            durationMs: 1,
            result: databaseResult,
          },
        });
      },
    });
    host.accept(
      executeV4(
        'inv_v4_postgres',
        `const db = services.db({ driver: 'postgres', connection: secrets.POSTGRES_URL });
        return await db.query({ text: 'SELECT $1::BOOL AS ready', values: [true] });`,
        { POSTGRES_URL: 'secret-value' }
      )
    );
    await host.drain();

    expect(calls).toHaveLength(1);
    expect(calls[0]?.call.input).toEqual({
      contract: 'woml.database',
      contractVersion: 1,
      kind: 'request',
      driver: 'postgres',
      connection: 'secret-value',
      operation: 'query',
      input: { text: 'SELECT $1::BOOL AS ready', values: [true] },
    });
    expect(completed[0]?.outcome).toEqual({
      kind: 'success',
      value: { rows: [{ ready: true }], rowCount: 1 },
    });
  });

  test('protocol v4 lowers services.storage operations and exposes only Storage v1 data', async () => {
    const completed: CompletedMessage[] = [];
    const calls: CapabilityCallMessage[] = [];
    let host!: ScriptHost;
    host = new ScriptHost({
      workerUrl: new URL('../src/script-host-worker.ts', import.meta.url),
      protocolVersion: 4,
      send: async message => {
        if (message.messageType === 'completed') {
          completed.push(message);
          return;
        }
        if (message.messageType !== 'capability_call') return;
        calls.push(message);
        const reference = {
          contract: 'woml.storage-object',
          contractVersion: 1,
          key: 'reports/today.json',
          version: `v1:${'a'.repeat(64)}`,
          checksum: { algorithm: 'sha256', value: 'b'.repeat(64) },
          size: 11,
          contentType: 'application/json',
        } as const;
        const storageResult = {
          contract: 'woml.storage',
          contractVersion: 1,
          kind: 'result',
          operation: message.call.operation,
          data: reference,
        } as const;
        host.accept({
          protocol: 'woml.script-host',
          protocolVersion: 4,
          messageType: 'capability_result',
          invocationId: message.invocationId,
          callId: message.callId,
          result: {
            contract: 'woml.capability-call',
            contractVersion: 1,
            messageType: 'result',
            invocationId: message.invocationId,
            callId: message.callId,
            outcome: 'succeeded',
            resultContractVersion: 1,
            resultBytes: Buffer.byteLength(JSON.stringify(storageResult)),
            durationMs: 1,
            result: storageResult,
          },
        });
      },
    });
    host.accept(
      executeV4(
        'inv_v4_storage',
        `const stored = await services.storage.put({
          key: 'reports/today.json', value: { total: 3 }
        }, { name: 'daily-report' });
        const head = await services.storage.head({ key: stored.key });
        return { stored, head, frozen: Object.isFrozen(services.storage) };`
      )
    );
    await host.drain();

    expect(calls).toHaveLength(2);
    expect(calls[0]?.call).toMatchObject({
      capability: 'storage',
      operation: 'put',
      identity: {
        mode: 'named',
        operationName: 'storage.put.daily-report',
      },
      input: {
        contract: 'woml.storage',
        contractVersion: 1,
        kind: 'request',
        operation: 'put',
        input: { key: 'reports/today.json', value: { total: 3 } },
      },
    });
    expect(calls[1]?.call).toMatchObject({
      capability: 'storage',
      operation: 'head',
      identity: { mode: 'automatic', operationName: 'storage.head' },
    });
    expect(completed[0]?.outcome).toMatchObject({
      kind: 'success',
      value: {
        stored: { key: 'reports/today.json' },
        head: { size: 11 },
        frozen: true,
      },
    });
  });

  test('protocol v4 normalizes managed HTTP direct-to-storage requests', async () => {
    const completed: CompletedMessage[] = [];
    const calls: CapabilityCallMessage[] = [];
    let host!: ScriptHost;
    host = new ScriptHost({
      workerUrl: new URL('../src/script-host-worker.ts', import.meta.url),
      protocolVersion: 4,
      send: async message => {
        if (message.messageType === 'completed') {
          completed.push(message);
          return;
        }
        if (message.messageType !== 'capability_call') return;
        calls.push(message);
        const managedResult = {
          contract: 'woml.managed-http',
          contractVersion: 1,
          kind: 'result',
          status: 200,
          ok: true,
          headers: { 'content-type': 'text/csv' },
          data: { key: 'imports/export.csv', size: 5_000_000 },
          url: 'https://files.example.test/export.csv',
          redirected: false,
        } as const;
        host.accept({
          protocol: 'woml.script-host',
          protocolVersion: 4,
          messageType: 'capability_result',
          invocationId: message.invocationId,
          callId: message.callId,
          result: {
            contract: 'woml.capability-call',
            contractVersion: 1,
            messageType: 'result',
            invocationId: message.invocationId,
            callId: message.callId,
            outcome: 'succeeded',
            resultContractVersion: 1,
            resultBytes: Buffer.byteLength(JSON.stringify(managedResult)),
            durationMs: 1,
            result: managedResult,
          },
        });
      },
    });
    host.accept(
      executeV4(
        'inv_v4_http_storage',
        `return await services.http.request({
          url: 'https://files.example.test/export.csv',
          responseType: 'storage',
          storage: { key: 'imports/export.csv', overwrite: true }
        });`
      )
    );
    await host.drain();

    expect(calls[0]?.call.input).toMatchObject({
      responseType: 'storage',
      storage: { key: 'imports/export.csv', overwrite: true },
    });
    expect(completed[0]?.outcome).toMatchObject({
      kind: 'success',
      value: { data: { key: 'imports/export.csv', size: 5_000_000 } },
    });
  });

  test('protocol v4 lowers positional services.cache calls to Cache v1', async () => {
    const completed: CompletedMessage[] = [];
    const calls: CapabilityCallMessage[] = [];
    let host!: ScriptHost;
    host = new ScriptHost({
      workerUrl: new URL('../src/script-host-worker.ts', import.meta.url),
      protocolVersion: 4,
      send: async message => {
        if (message.messageType === 'completed') {
          completed.push(message);
          return;
        }
        if (message.messageType !== 'capability_call') return;
        calls.push(message);
        const data =
          message.call.operation === 'set'
            ? { stored: true, expiresAt: '2026-08-10T12:00:00.000Z' }
            : message.call.operation === 'get'
              ? {
                  hit: true,
                  value: { name: 'Ada' },
                  expiresAt: '2026-08-10T12:00:00.000Z',
                }
              : {
                  stored: false,
                  value: { name: 'Ada' },
                  expiresAt: '2026-08-10T12:00:00.000Z',
                };
        const cacheResult = {
          contract: 'woml.cache',
          contractVersion: 1,
          kind: 'result',
          operation: message.call.operation,
          data,
        } as const;
        host.accept({
          protocol: 'woml.script-host',
          protocolVersion: 4,
          messageType: 'capability_result',
          invocationId: message.invocationId,
          callId: message.callId,
          result: {
            contract: 'woml.capability-call',
            contractVersion: 1,
            messageType: 'result',
            invocationId: message.invocationId,
            callId: message.callId,
            outcome: 'succeeded',
            resultContractVersion: 1,
            resultBytes: Buffer.byteLength(JSON.stringify(cacheResult)),
            durationMs: 1,
            result: cacheResult,
          },
        });
      },
    });
    host.accept(
      executeV4(
        'inv_v4_cache',
        `const stored = await services.cache.set('customer:42', { name: 'Ada' }, {
          ttl: '15m', name: 'cache-customer'
        });
        const loaded = await services.cache.get('customer:42');
        const existing = await services.cache.setIfAbsent('customer:42', { name: 'Grace' }, {
          ttl: '1h', name: 'initialize-customer'
        });
        return { stored, loaded, existing, frozen: Object.isFrozen(services.cache) };`
      )
    );
    await host.drain();

    expect(calls).toHaveLength(3);
    expect(calls[0]?.call).toMatchObject({
      capability: 'cache',
      operation: 'set',
      identity: {
        mode: 'named',
        operationName: 'cache.set.cache-customer',
      },
      input: {
        contract: 'woml.cache',
        contractVersion: 1,
        kind: 'request',
        operation: 'set',
        input: {
          key: 'customer:42',
          value: { name: 'Ada' },
          ttlMs: 900_000,
        },
      },
    });
    expect(calls[1]?.call).toMatchObject({
      capability: 'cache',
      operation: 'get',
      input: { operation: 'get', input: { key: 'customer:42' } },
    });
    expect(calls[2]?.call).toMatchObject({
      capability: 'cache',
      operation: 'set_if_absent',
      identity: {
        mode: 'named',
        operationName: 'cache.set_if_absent.initialize-customer',
      },
      input: {
        operation: 'set_if_absent',
        input: { ttlMs: 3_600_000 },
      },
    });
    expect(completed[0]?.outcome).toMatchObject({
      kind: 'success',
      value: {
        loaded: { hit: true, value: { name: 'Ada' } },
        existing: { stored: false, value: { name: 'Ada' } },
        frozen: true,
      },
    });
  });

  test('protocol v4 bindings are deeply read-only and reject a known secret in results', async () => {
    const sent: CompletedMessage[] = [];
    const host = new ScriptHost({
      workerUrl: new URL('../src/script-host-worker.ts', import.meta.url),
      protocolVersion: 4,
      send: async message => {
        if (message.messageType === 'completed') sent.push(message);
      },
    });
    host.accept(
      executeV4(
        'inv_v4_secret',
        `return {
          frozen: Object.isFrozen(secrets) && Object.isFrozen(services),
          leaked: 'prefix-' + secrets.API_TOKEN
        };`,
        { API_TOKEN: 'secret-v4-value' }
      )
    );
    await host.drain();
    expect(sent[0]?.outcome).toEqual({
      kind: 'failure',
      error: {
        kind: 'invalid_script_result',
        code: 'WOML_SCRIPT_NON_JSON_RESULT',
        message: 'Script results must not contain a resolved secret value.',
      },
    });
    expect(JSON.stringify(sent)).not.toContain('secret-v4-value');
  });

  test('protocol v4 preserves native Fetch behavior while redacting durable observations', async () => {
    const requests: Array<{
      url: string;
      authorization: string;
      body: string;
    }> = [];
    let serverBase = '';
    const server = Bun.serve({
      hostname: '127.0.0.1',
      port: 0,
      async fetch(request): Promise<Response> {
        const url = new URL(request.url);
        if (url.pathname === '/redirect') {
          return Response.redirect(`${serverBase}/final`, 302);
        }
        if (url.pathname === '/final') {
          return new Response(Uint8Array.from([0, 1, 2, 255]), {
            status: 206,
            headers: { 'x-woml-test': 'native' },
          });
        }
        requests.push({
          url: request.url,
          authorization: request.headers.get('authorization') ?? '',
          body: await request.text(),
        });
        return new Response('accepted', { status: 202 });
      },
    });
    serverBase = server.url.toString().replace(/\/$/, '');
    try {
      const completed: CompletedMessage[] = [];
      const observations: FetchObservationMessage[] = [];
      let host!: ScriptHost;
      host = new ScriptHost({
        workerUrl: new URL('../src/script-host-worker.ts', import.meta.url),
        protocolVersion: 4,
        send: async message => {
          if (message.messageType === 'completed') {
            completed.push(message);
            return;
          }
          if (message.messageType !== 'fetch_observation') return;
          observations.push(message);
          host.accept({
            protocol: 'woml.script-host',
            protocolVersion: 4,
            messageType: 'fetch_observation_ack',
            invocationId: message.invocationId,
            requestId: message.requestId,
            accepted: true,
          });
        },
      });
      const base = serverBase;
      host.accept(
        executeV4(
          'inv_v4_fetch_native',
          `const request = new Request(${JSON.stringify(`${base}/echo?access_token=hidden`)}, {
            method: 'POST',
            headers: new Headers({ authorization: 'Bearer secret-header' }),
            body: 'Héllo 🌍\\r\\nbody'
          });
          const response = await fetch(request);
          const redirected = await globalThis.fetch(${JSON.stringify(`${base}/redirect`)});
          const concurrent = await Promise.all([
            fetch(${JSON.stringify(`${base}/final`)}),
            fetch(${JSON.stringify(`${base}/final`)})
          ]);
          const localData = await (await fetch('data:text/plain,native-data')).text();
          return {
            responseIsNative: response instanceof Response,
            status: response.status,
            text: await response.text(),
            redirected: redirected.redirected,
            redirectStatus: redirected.status,
            header: redirected.headers.get('x-woml-test'),
            bytes: [...new Uint8Array(await redirected.arrayBuffer())],
            concurrentStatuses: concurrent.map(item => item.status),
            localData
          };`
        )
      );
      await host.drain();

      expect(completed[0]?.outcome).toEqual({
        kind: 'success',
        value: {
          responseIsNative: true,
          status: 202,
          text: 'accepted',
          redirected: true,
          redirectStatus: 206,
          header: 'native',
          bytes: [0, 1, 2, 255],
          concurrentStatuses: [206, 206],
          localData: 'native-data',
        },
      });
      expect(requests).toEqual([
        {
          url: `${base}/echo?access_token=hidden`,
          authorization: 'Bearer secret-header',
          body: 'Héllo 🌍\r\nbody',
        },
      ]);
      expect(observations).toHaveLength(8);
      expect(observations[0]?.observation).toMatchObject({
        observationType: 'started',
        method: 'POST',
        origin: new URL(base).origin,
        path: '/echo',
      });
      expect(observations[1]?.observation).toMatchObject({
        observationType: 'completed',
        status: 202,
        responseBodyBytes: null,
      });
      const durableText = JSON.stringify(observations);
      expect(durableText).not.toContain('access_token');
      expect(durableText).not.toContain('secret-header');
      expect(durableText).not.toContain('Héllo');
    } finally {
      server.stop(true);
    }
  });

  test('protocol v4 refuses to dispatch native Fetch when start tracking is rejected', async () => {
    const completed: CompletedMessage[] = [];
    let host!: ScriptHost;
    host = new ScriptHost({
      workerUrl: new URL('../src/script-host-worker.ts', import.meta.url),
      protocolVersion: 4,
      send: async message => {
        if (message.messageType === 'completed') {
          completed.push(message);
          return;
        }
        if (message.messageType !== 'fetch_observation') return;
        host.accept({
          protocol: 'woml.script-host',
          protocolVersion: 4,
          messageType: 'fetch_observation_ack',
          invocationId: message.invocationId,
          requestId: message.requestId,
          accepted: false,
          error: {
            kind: 'transport_failed',
            code: 'WOML_NATIVE_FETCH_TRACKING_FAILED',
            message: 'Durable Fetch tracking is unavailable.',
            retryable: false,
            ambiguous: false,
          },
        });
      },
    });
    host.accept(
      executeV4(
        'inv_v4_fetch_rejected',
        `await fetch('http://127.0.0.1:9/must-not-dispatch'); return true;`
      )
    );
    await host.drain();
    expect(completed[0]?.outcome).toMatchObject({
      kind: 'failure',
      error: {
        kind: 'service_failed',
        message: 'Durable Fetch tracking is unavailable.',
        capability: 'http',
        operation: 'fetch',
        retryable: false,
        ambiguous: false,
      },
    });
  });

  test('protocol v4 classifies an uncaught native Fetch rejection as non-retryable', async () => {
    const completed: CompletedMessage[] = [];
    let host!: ScriptHost;
    host = new ScriptHost({
      workerUrl: new URL('../src/script-host-worker.ts', import.meta.url),
      protocolVersion: 4,
      send: async message => {
        if (message.messageType === 'completed') {
          completed.push(message);
          return;
        }
        if (message.messageType !== 'fetch_observation') return;
        host.accept({
          protocol: 'woml.script-host',
          protocolVersion: 4,
          messageType: 'fetch_observation_ack',
          invocationId: message.invocationId,
          requestId: message.requestId,
          accepted: true,
        });
      },
    });
    host.accept(
      executeV4(
        'inv_v4_fetch_uncaught',
        `await fetch('http://127.0.0.1:1/unavailable'); return true;`
      )
    );
    await host.drain();
    expect(completed[0]?.outcome).toMatchObject({
      kind: 'failure',
      error: {
        kind: 'service_failed',
        code: 'WOML_NATIVE_FETCH_REJECTED',
        capability: 'http',
        operation: 'fetch',
        retryable: false,
        ambiguous: true,
      },
    });
  });

  test('protocol v4 cancellation terminates a Worker with active native Fetch exactly once', async () => {
    const server = Bun.serve({
      hostname: '127.0.0.1',
      port: 0,
      fetch: async () => {
        await Bun.sleep(2_000);
        return new Response('late');
      },
    });
    try {
      const completed: CompletedMessage[] = [];
      const observations: FetchObservationMessage[] = [];
      let host!: ScriptHost;
      host = new ScriptHost({
        workerUrl: new URL('../src/script-host-worker.ts', import.meta.url),
        protocolVersion: 4,
        send: async message => {
          if (message.messageType === 'completed') {
            completed.push(message);
            return;
          }
          if (message.messageType !== 'fetch_observation') return;
          observations.push(message);
          host.accept({
            protocol: 'woml.script-host',
            protocolVersion: 4,
            messageType: 'fetch_observation_ack',
            invocationId: message.invocationId,
            requestId: message.requestId,
            accepted: true,
          });
        },
      });
      host.accept(
        executeV4(
          'inv_v4_fetch_cancel',
          `return await fetch(${JSON.stringify(server.url.toString())});`
        )
      );
      while (observations.length === 0) await Bun.sleep(1);
      host.accept({
        protocol: 'woml.script-host',
        protocolVersion: 4,
        messageType: 'cancel',
        invocationId: 'inv_v4_fetch_cancel',
        reason: 'run_cancelled',
      });
      await host.drain();
      expect(completed).toHaveLength(1);
      expect(completed[0]?.outcome).toMatchObject({
        kind: 'failure',
        error: { kind: 'invocation_cancelled' },
      });
      expect(observations).toHaveLength(1);
      expect(observations[0]?.observation.observationType).toBe('started');
    } finally {
      server.stop(true);
    }
  });

  test('protocol v4 cancellation drops a known late reply and rejects an unknown reply', async () => {
    const completed: CompletedMessage[] = [];
    let captured: CapabilityCallMessage | undefined;
    const host = new ScriptHost({
      workerUrl: new URL('../src/script-host-worker.ts', import.meta.url),
      protocolVersion: 4,
      send: async message => {
        if (message.messageType === 'completed') completed.push(message);
        else if (message.messageType === 'capability_call') captured = message;
      },
    });
    host.accept(
      executeV4(
        'inv_v4_cancel',
        `return await services.test.control({ mode: 'delay', delayMs: 1000 });`
      )
    );
    while (captured === undefined) await Bun.sleep(1);
    host.accept({
      protocol: 'woml.script-host',
      protocolVersion: 4,
      messageType: 'cancel',
      invocationId: 'inv_v4_cancel',
      reason: 'run_cancelled',
    });
    await host.drain();
    expect(completed[0]?.outcome).toMatchObject({
      kind: 'failure',
      error: { kind: 'invocation_cancelled' },
    });

    const call = captured!;
    const lateResult: CapabilityResultMessage = {
      protocol: 'woml.script-host',
      protocolVersion: 4,
      messageType: 'capability_result',
      invocationId: call.invocationId,
      callId: call.callId,
      result: {
        contract: 'woml.capability-call',
        contractVersion: 1,
        messageType: 'result',
        invocationId: call.invocationId,
        callId: call.callId,
        outcome: 'succeeded',
        resultContractVersion: 1,
        resultBytes: 4,
        durationMs: 1,
        result: null,
      },
    };
    expect(() => host.accept(lateResult)).not.toThrow();

    expect(() =>
      host.accept({
        protocol: 'woml.script-host',
        protocolVersion: 4,
        messageType: 'capability_result',
        invocationId: call.invocationId,
        callId: 'call_never_existed',
        result: {
          contract: 'woml.capability-call',
          contractVersion: 1,
          messageType: 'result',
          invocationId: call.invocationId,
          callId: 'call_never_existed',
          outcome: 'succeeded',
          resultContractVersion: 1,
          resultBytes: 4,
          durationMs: 1,
          result: null,
        },
      })
    ).toThrow('unknown call ID');
  });
});

describe('MS3 Script Host v5 module runtime', () => {
  test('registers one immutable bundle and creates fresh sync/async module state per Worker', async () => {
    const bundle = `let calls = 0;
export function sync() { calls += 1; return calls; }
export async function asyncValue() { calls += 1; return calls; }`;
    const digest = moduleDigest(bundle);
    const source = `return {
      sync: services.utility.sync(),
      async: await services.utility.asyncValue()
    };`;
    const result = await runHost(
      [
        {
          protocol: 'woml.script-host',
          protocolVersion: 5,
          messageType: 'register_module',
          bundleDigest: digest,
          bundle,
        },
        executeV5('inv_module_first', source, digest, ['asyncValue', 'sync']),
        executeV5('inv_module_second', source, digest, ['asyncValue', 'sync']),
      ],
      { WOML_SCRIPT_HOST_PROTOCOL_VERSION: '5' }
    );
    expect(result.exitCode).toBe(0);
    expect(result.messages[1]).toEqual({
      protocol: 'woml.script-host',
      protocolVersion: 5,
      messageType: 'module_registered',
      bundleDigest: digest,
      accepted: true,
    });
    expect(byInvocation(result.messages).get('inv_module_first')).toMatchObject(
      {
        outcome: { kind: 'success', value: { sync: 1, async: 2 } },
      }
    );
    expect(
      byInvocation(result.messages).get('inv_module_second')
    ).toMatchObject({
      outcome: { kind: 'success', value: { sync: 1, async: 2 } },
    });
  });

  test('rejects effects during initialization and keeps imported namespaces read-only', async () => {
    const effectBundle = `await fetch('https://example.invalid');
export function value() { return 1; }`;
    const effectDigest = moduleDigest(effectBundle);
    const readonlyBundle = `export function value() { return 1; }`;
    const readonlyDigest = moduleDigest(readonlyBundle);
    const result = await runHost(
      [
        {
          protocol: 'woml.script-host',
          protocolVersion: 5,
          messageType: 'register_module',
          bundleDigest: effectDigest,
          bundle: effectBundle,
        },
        executeV5(
          'inv_module_initialization',
          'return services.utility.value();',
          effectDigest,
          ['value']
        ),
        {
          protocol: 'woml.script-host',
          protocolVersion: 5,
          messageType: 'register_module',
          bundleDigest: readonlyDigest,
          bundle: readonlyBundle,
        },
        executeV5(
          'inv_module_readonly',
          'services.utility.value = () => 2; return true;',
          readonlyDigest,
          ['value']
        ),
      ],
      { WOML_SCRIPT_HOST_PROTOCOL_VERSION: '5' }
    );
    expect(
      byInvocation(result.messages).get('inv_module_initialization')
    ).toMatchObject({
      outcome: {
        kind: 'failure',
        error: {
          kind: 'script_threw',
          message: expect.stringContaining(
            'cannot be used while a WOML module is initializing'
          ),
        },
      },
    });
    expect(
      byInvocation(result.messages).get('inv_module_readonly')
    ).toMatchObject({
      outcome: { kind: 'failure', error: { kind: 'script_threw' } },
    });
  });

  test('rejects tampered registration and applies the existing step timeout to module calls', async () => {
    const bundle = `export function never() { while (true) {} }`;
    const digest = moduleDigest(bundle);
    const timed = executeV5(
      'inv_module_timeout',
      'return services.utility.never();',
      digest,
      ['never']
    );
    const result = await runHost(
      [
        {
          protocol: 'woml.script-host',
          protocolVersion: 5,
          messageType: 'register_module',
          bundleDigest: defaultEffectKey,
          bundle: 'export function tampered() {}',
        },
        {
          protocol: 'woml.script-host',
          protocolVersion: 5,
          messageType: 'register_module',
          bundleDigest: digest,
          bundle,
        },
        { ...timed, timeoutMs: 25 },
      ],
      { WOML_SCRIPT_HOST_PROTOCOL_VERSION: '5' }
    );
    expect(result.messages).toContainEqual({
      protocol: 'woml.script-host',
      protocolVersion: 5,
      messageType: 'module_registered',
      bundleDigest: defaultEffectKey,
      accepted: false,
      code: 'WOML_MODULE_DIGEST_MISMATCH',
      message:
        'The registered module bundle does not match its SHA-256 identity.',
    });
    expect(
      byInvocation(result.messages).get('inv_module_timeout')
    ).toMatchObject({
      outcome: { kind: 'failure', error: { kind: 'script_timed_out' } },
    });
  });
});

describe('MS4 Script Host v6 recoverability and diagnostics', () => {
  test('re-registers immutable artifacts after a host restart', async () => {
    const bundle = `export function value() { return { recovered: true }; }`;
    const sourceMap = JSON.stringify({
      version: 3,
      sources: ['modules/utility.ts'],
      sourcesContent: ['export function value() {}'],
      names: [],
      mappings: '',
    });
    const digest = moduleDigest(bundle);
    const sourceMapDigest = moduleDigest(sourceMap);
    const messages = [
      {
        protocol: 'woml.script-host',
        protocolVersion: 6,
        messageType: 'register_module',
        bundleDigest: digest,
        bundle,
        sourceMapDigest,
        sourceMap,
      },
      executeV6(
        'inv_ms4_recovered',
        'return services.utility.value();',
        digest,
        ['value']
      ),
    ] as const;

    for (let restart = 0; restart < 2; restart += 1) {
      const result = await runHost(messages, {
        WOML_SCRIPT_HOST_PROTOCOL_VERSION: '6',
      });
      expect(result.exitCode).toBe(0);
      expect(result.messages[1]).toEqual({
        protocol: 'woml.script-host',
        protocolVersion: 6,
        messageType: 'module_registered',
        bundleDigest: digest,
        sourceMapDigest,
        accepted: true,
      });
      expect(
        byInvocation(result.messages).get('inv_ms4_recovered')
      ).toMatchObject({
        outcome: { kind: 'success', value: { recovered: true } },
      });
    }
  });

  test('reports a safe module location and redacts resolved secrets', async () => {
    const secret = 'ms4-super-secret';
    const bundle = `export function fail(value) {
  throw new Error('failed with ' + value);
}
//# sourceURL=modules/utility.ts`;
    const sourceMap = JSON.stringify({
      version: 3,
      sources: ['modules/utility.ts'],
      sourcesContent: [
        `export function fail(value: string) {
  throw new Error('failed with ' + value);
}`,
      ],
      names: [],
      mappings: 'AAAA;AACA;AACA',
    });
    const digest = moduleDigest(bundle);
    const sourceMapDigest = moduleDigest(sourceMap);
    const result = await runHost(
      [
        {
          protocol: 'woml.script-host',
          protocolVersion: 6,
          messageType: 'register_module',
          bundleDigest: digest,
          bundle,
          sourceMapDigest,
          sourceMap,
        },
        executeV6(
          'inv_ms4_diagnostic',
          'return services.utility.fail(secrets.API_TOKEN);',
          digest,
          ['fail'],
          { API_TOKEN: secret }
        ),
      ],
      { WOML_SCRIPT_HOST_PROTOCOL_VERSION: '6' }
    );
    const completion = byInvocation(result.messages).get('inv_ms4_diagnostic');
    const diagnostic = JSON.stringify(completion);
    expect(completion).toMatchObject({
      outcome: {
        kind: 'failure',
        error: {
          kind: 'script_threw',
          message: expect.stringContaining('[REDACTED]'),
        },
      },
    });
    expect(JSON.stringify(result.messages)).not.toContain(secret);
    expect(diagnostic).toContain('modules/utility.ts:');
  });

  test('rejects a tampered source map and an oversized artifact before execution', async () => {
    const bundle = `export function value() { return true; }`;
    const digest = moduleDigest(bundle);
    const result = await runHost(
      [
        {
          protocol: 'woml.script-host',
          protocolVersion: 6,
          messageType: 'register_module',
          bundleDigest: digest,
          bundle,
          sourceMapDigest: defaultEffectKey,
          sourceMap: '{}',
        },
      ],
      { WOML_SCRIPT_HOST_PROTOCOL_VERSION: '6' }
    );
    expect(result.messages[1]).toMatchObject({
      accepted: false,
      code: 'WOML_MODULE_DIGEST_MISMATCH',
    });
    const directHost = new ScriptHost({
      workerUrl: new URL('../src/script-host-worker.ts', import.meta.url),
      protocolVersion: 6,
      send: async () => {},
    });
    const oversized = 'x'.repeat(3 * 1024 * 1024 + 1);
    expect(() =>
      directHost.accept({
        protocol: 'woml.script-host',
        protocolVersion: 6,
        messageType: 'register_module',
        bundleDigest: moduleDigest(oversized),
        bundle: oversized,
        sourceMapDigest: moduleDigest('{}'),
        sourceMap: '{}',
      })
    ).toThrow('protocol v6');
  });
});

describe('LEC3 Script Host v7 lifecycle mode', () => {
  test('injects a deeply read-only lifecycle binding and accepts an undefined return', async () => {
    const result = await runHost(
      [
        executeLifecycleV7(
          'inv_lifecycle_binding',
          `
            if (!Object.isFrozen(lifecycle) || !Object.isFrozen(lifecycle.workflow)) {
              throw new Error('lifecycle was mutable');
            }
            if (context.trigger.orderId !== 'order-1') throw new Error('context missing');
            try { context.steps.injected = true; } catch {}
          `
        ),
      ],
      { WOML_SCRIPT_HOST_PROTOCOL_VERSION: '7' }
    );
    const completed = result.messages.find(
      message => message.messageType === 'completed'
    ) as CompletedMessage;
    expect(completed.outcome).toEqual({ kind: 'success', value: null });
  });

  test('keeps lifecycle throw, timeout, and non-JSON failures distinct', async () => {
    const result = await runHost(
      [
        executeLifecycleV7('inv_lifecycle_throw', `throw new Error('boom');`),
        executeLifecycleV7('inv_lifecycle_non_json', `return () => true;`),
        {
          ...executeLifecycleV7(
            'inv_lifecycle_timeout',
            `await new Promise(resolve => setTimeout(resolve, 500));`
          ),
          timeoutMs: 20,
        },
      ],
      { WOML_SCRIPT_HOST_PROTOCOL_VERSION: '7' }
    );
    const failures = result.messages
      .filter(message => message.messageType === 'completed')
      .map(message => (message as CompletedMessage).outcome)
      .filter(outcome => outcome.kind === 'failure')
      .map(outcome => outcome.error.kind)
      .sort();
    expect(failures).toEqual([
      'invalid_script_result',
      'script_threw',
      'script_timed_out',
    ]);
  });
});

describe('DS1 Script Host v7 / DS3 managed Durable User State', () => {
  test('is deeply read-only and lowers calls to the frozen State v1 contract', async () => {
    const completed: CompletedMessage[] = [];
    const calls: CapabilityCallMessage[] = [];
    let host!: ScriptHost;
    host = new ScriptHost({
      workerUrl: new URL('../src/script-host-worker.ts', import.meta.url),
      protocolVersion: 7,
      send: async message => {
        if (message.messageType === 'completed') {
          completed.push(message);
          return;
        }
        if (message.messageType !== 'capability_call') return;
        calls.push(message);
        const stateResult = {
          contract: 'woml.state',
          contractVersion: 1,
          kind: 'result',
          operation: message.call.operation,
          data: {
            stored: true,
            version: 1,
            updatedAt: '2026-08-12T10:00:00.000Z',
          },
        } as const;
        host.accept({
          protocol: 'woml.script-host',
          protocolVersion: 7,
          messageType: 'capability_result',
          invocationId: message.invocationId,
          callId: message.callId,
          result: {
            contract: 'woml.capability-call',
            contractVersion: 1,
            messageType: 'result',
            invocationId: message.invocationId,
            callId: message.callId,
            outcome: 'succeeded',
            resultContractVersion: 1,
            resultBytes: Buffer.byteLength(JSON.stringify(stateResult)),
            durationMs: 1,
            result: stateResult,
          },
        });
      },
    });
    host.accept(
      executeStepV7(
        'inv_ds1_state',
        `
          if (!Object.isFrozen(services.state)) throw new Error('state facade was mutable');
          if (!Object.isFrozen(services.state.set)) throw new Error('state method was mutable');
          const saved = await services.state.set('conversation:C123', { count: 1 }, {
            name: 'remember-conversation',
            ifVersion: 0
          });
          return { saved, frozen: Object.isFrozen(saved) };
        `
      )
    );
    await host.drain();

    expect(calls).toHaveLength(1);
    expect(calls[0]?.call).toMatchObject({
      capability: 'state',
      operation: 'set',
      identity: {
        mode: 'named',
        operationName: 'state.set.remember-conversation',
      },
      input: {
        contract: 'woml.state',
        contractVersion: 1,
        kind: 'request',
        operation: 'set',
        input: {
          key: 'conversation:C123',
          value: { count: 1 },
          ifVersion: 0,
        },
      },
    });
    expect(completed[0]?.outcome).toMatchObject({
      kind: 'success',
      value: {
        saved: { stored: true, version: 1 },
        frozen: true,
      },
    });
  });

  test('rejects invalid keys, values, versions, names, and argument counts locally', async () => {
    const cases = [
      `await services.state.get('');`,
      `await services.state.set('key', undefined, { name: 'write' });`,
      `await services.state.set('key', 1, { name: 'Write Bad' });`,
      `await services.state.delete('key', { name: 'delete', ifVersion: -1 });`,
      `await services.state.increment('key', 1.5, { name: 'increment' });`,
      `await services.state.setIfAbsent('key', 1);`,
    ];
    const result = await runHost(
      cases.map((source, index) => executeStepV7(`inv_ds1_invalid_${index}`, source)),
      { WOML_SCRIPT_HOST_PROTOCOL_VERSION: '7' }
    );
    const completed = result.messages.filter(
      message => message.messageType === 'completed'
    ) as CompletedMessage[];
    expect(completed).toHaveLength(cases.length);
    expect(completed.every(message => message.outcome.kind === 'failure')).toBe(true);
    expect(result.messages.some(message => message.messageType === 'capability_call')).toBe(false);
  });
});
