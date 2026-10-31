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
  ExecuteMessageV8,
  ExecuteMessageV9,
  ExecuteMessageV10,
  ExecuteMessageV11,
  ExecuteMessageV12,
  FetchObservationMessage,
  JsonValue,
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

function executeReusableV8(
  invocationId: string,
  source: string,
  props: Readonly<Record<string, JsonValue>>,
  secrets: Readonly<Record<string, string>> = {}
): ExecuteMessageV8 {
  return {
    ...executeStepV7(invocationId, source),
    protocolVersion: 8,
    bindings: { bindingVersion: 1, servicesVersion: 1, secrets },
    reusable: {
      profile: 'woml.reusable-script-binding/v3',
      invocationId: 'discount',
      definition: {
        kind: 'step',
        alias: 'calculate-discount',
        digest: `sha256:${'1'.repeat(64)}`,
        source: 'calculate-discount.woml',
      },
      props,
    },
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

  test('keeps step console output off protocol stdout and redacts secrets', async () => {
    const secret = 'console-secret-must-not-leak';
    const message: ExecuteMessageV8 = {
      ...executeStepV7(
        'inv_safe_step_console',
        `console.log('Step log', secrets.API_TOKEN); return { logged: true };`
      ),
      protocolVersion: 8,
      bindings: {
        bindingVersion: 1,
        servicesVersion: 1,
        secrets: { API_TOKEN: secret },
      },
    };
    const result = await runHost([message], {
      WOML_SCRIPT_HOST_PROTOCOL_VERSION: '8',
    });

    expect(result.exitCode).toBe(0);
    expect(
      byInvocation(result.messages).get(message.invocationId)?.outcome
    ).toEqual({
      kind: 'success',
      value: { logged: true },
    });
    expect(result.stderr).toContain('Step log [REDACTED]');
    expect(result.stderr).not.toContain(secret);
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

  test('protocol v4 lowers services.telegram.send to one supervised capability call', async () => {
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
          provider: 'telegram',
          conversationId: '-1001234567890',
          messageId: '42',
          acceptedAt: '2026-08-16T12:00:00.000Z',
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
            durationMs: 2,
            result: managedResult,
          },
        });
      },
    });
    host.accept(
      executeV4(
        'inv_v4_telegram_send',
        `return services.telegram.send({
          botToken: 'synthetic-token',
          conversationId: '-1001234567890',
          text: 'Hello from WOML',
          replyToMessageId: '41'
        }, { name: 'reply-to-message' });`
      )
    );
    await host.drain();

    expect(calls).toHaveLength(1);
    expect(calls[0]?.call).toMatchObject({
      capability: 'telegram',
      operation: 'send',
      identity: {
        mode: 'named',
        operationName: 'telegram.send.reply-to-message',
      },
      input: {
        contract: 'woml.telegram-message',
        contractVersion: 1,
        kind: 'send',
        botToken: 'synthetic-token',
        conversationId: '-1001234567890',
        text: 'Hello from WOML',
        replyToMessageId: '41',
      },
    });
    expect(completed[0]?.outcome).toEqual({
      kind: 'success',
      value: {
        provider: 'telegram',
        conversationId: '-1001234567890',
        messageId: '42',
        acceptedAt: '2026-08-16T12:00:00.000Z',
      },
    });
  });

  test('protocol v4 lowers services.discord.send to one supervised capability call', async () => {
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
          provider: 'discord',
          conversationId: '345678901234567890',
          messageId: '456789012345678901',
          acceptedAt: '2026-08-16T12:00:00.000Z',
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
            durationMs: 2,
            result: managedResult,
          },
        });
      },
    });
    host.accept(
      executeV4(
        'inv_v4_discord_send',
        `return services.discord.send({
          botToken: 'synthetic-token',
          conversationId: '345678901234567890',
          text: 'Hello from WOML',
          replyToMessageId: '456789012345678900'
        }, { name: 'reply-to-message' });`
      )
    );
    await host.drain();

    expect(calls).toHaveLength(1);
    expect(calls[0]?.call).toMatchObject({
      capability: 'discord',
      operation: 'send',
      identity: {
        mode: 'named',
        operationName: 'discord.send.reply-to-message',
      },
      input: {
        contract: 'woml.discord-message',
        contractVersion: 1,
        kind: 'send',
        botToken: 'synthetic-token',
        conversationId: '345678901234567890',
        text: 'Hello from WOML',
        replyToMessageId: '456789012345678900',
      },
    });
    expect(completed[0]?.outcome).toEqual({
      kind: 'success',
      value: {
        provider: 'discord',
        conversationId: '345678901234567890',
        messageId: '456789012345678901',
        acceptedAt: '2026-08-16T12:00:00.000Z',
      },
    });
  });

  test('Lowers workflows.call, bounds waiting, and enforces stable repeated identities', async () => {
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

  test('Process v1 normalizes one supervised call and exposes an immutable public result', async () => {
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
        const processResult = {
          contract: 'woml.process',
          contractVersion: 1,
          kind: 'result',
          exitCode: 0,
          signal: null,
          stdout: 'ok top-secret-value then top-secr',
          stderr: '',
          stdoutBytes: 33,
          stderrBytes: 0,
          stdoutTruncated: true,
          stderrTruncated: false,
          durationMs: 12.5,
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
            resultBytes: Buffer.byteLength(JSON.stringify(processResult)),
            durationMs: 13,
            result: processResult,
          },
        });
      },
    });

    host.accept(
      executeV4(
        'inv_v4_process',
        `
          const facadeFrozen = Object.isFrozen(services.process);
          const methodFrozen = Object.isFrozen(services.process.run);
          const value = await services.process.run({
            executable: 'bun',
            args: ['test', '--watch=false'],
            cwd: './packages/runtime',
            env: { CI: 'true', API_TOKEN: secrets.API_TOKEN },
            timeout: '5s',
            acceptExitCodes: [0, 2],
            output: { stdout: 'capture', stderr: 'discard', maxBytes: 4096 }
          }, { name: 'project-tests' });
          return {
            facadeFrozen,
            methodFrozen,
            resultFrozen: Object.isFrozen(value),
            hasContractField: Object.hasOwn(value, 'contract'),
            value
          };
        `,
        { API_TOKEN: 'top-secret-value' }
      )
    );
    await host.drain();

    expect(calls).toHaveLength(1);
    expect(calls[0]?.call).toMatchObject({
      capability: 'process',
      operation: 'run',
      inputContractVersion: 1,
      resultContractVersion: 1,
      identity: {
        mode: 'named',
        operationName: 'process.run.project-tests',
      },
      input: {
        contract: 'woml.process',
        contractVersion: 1,
        kind: 'request',
        executable: 'bun',
        args: ['test', '--watch=false'],
        cwd: './packages/runtime',
        env: { CI: 'true', API_TOKEN: 'top-secret-value' },
        acceptExitCodes: [0, 2],
        output: {
          stdout: 'capture',
          stderr: 'discard',
          maxBytes: 4096,
        },
      },
    });
    expect(calls[0]!.call.limits.timeoutMs).toBeGreaterThan(0);
    expect(Number((calls[0]!.call.input as Record<string, unknown>).timeoutMs)).toBeLessThanOrEqual(
      2_000
    );
    expect(calls[0]!.call.limits.timeoutMs).toBeLessThanOrEqual(2_000);
    expect(calls[0]!.call.identity.operationKey).toMatch(/^sha256:[a-f0-9]{64}$/);
    expect(completed[0]?.outcome).toEqual({
      kind: 'success',
      value: {
        facadeFrozen: true,
        methodFrozen: true,
        resultFrozen: true,
        hasContractField: false,
        value: {
          exitCode: 0,
          signal: null,
          stdout: 'ok [REDACTED] then ********',
          stderr: '',
          stdoutBytes: 33,
          stderrBytes: 0,
          stdoutTruncated: true,
          stderrTruncated: false,
          durationMs: 12.5,
        },
      },
    });
  });

  test('Process v2 exposes a verified immutable rollover artifact reference', async () => {
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
        const processResult = {
          contract: 'woml.process',
          contractVersion: 2,
          kind: 'result',
          exitCode: 0,
          signal: null,
          stdout: '',
          stderr: '',
          stdoutBytes: 20_000_000,
          stderrBytes: 0,
          stdoutTruncated: true,
          stderrTruncated: false,
          durationMs: 125,
          streamed: true,
          streamStartSequence: 1,
          streamEndSequence: 1_024,
          streamTruncated: true,
          streamArtifact: {
            contract: 'woml.artifact-reference',
            contractVersion: 1,
            id: 'artifact_01j00000000000000000000000',
            name: 'process-output.ndjson',
            size: 20_000_000,
            sha256: 'a'.repeat(64),
            contentType: 'application/x-ndjson',
          },
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
            resultBytes: Buffer.byteLength(JSON.stringify(processResult)),
            durationMs: 126,
            result: processResult,
          },
        });
      },
    });

    host.accept(
      executeV4(
        'inv_v4_process_streaming',
        `
          const value = await services.process.run({
            executable: 'bun', args: ['run', 'build'], stream: true
          }, { name: 'stream-build' });
          return {
            artifactFrozen: Object.isFrozen(value.streamArtifact),
            artifact: value.streamArtifact
          };
        `
      )
    );
    await host.drain();

    expect(calls).toHaveLength(1);
    expect(calls[0]?.call).toMatchObject({
      capability: 'process',
      operation: 'run',
      inputContractVersion: 1,
      resultContractVersion: 1,
      input: { contractVersion: 2, stream: true },
    });
    expect(completed[0]?.outcome).toMatchObject({
      kind: 'success',
      value: {
        artifactFrozen: true,
        artifact: {
          contract: 'woml.artifact-reference',
          contractVersion: 1,
          id: 'artifact_01j00000000000000000000000',
          sha256: 'a'.repeat(64),
        },
      },
    });
  });

  test('Process v1 rejects unsafe and malformed requests before a capability call', async () => {
    const completed: CompletedMessage[] = [];
    let callCount = 0;
    const host = new ScriptHost({
      workerUrl: new URL('../src/script-host-worker.ts', import.meta.url),
      protocolVersion: 4,
      send: async message => {
        if (message.messageType === 'completed') completed.push(message);
        if (message.messageType === 'capability_call') callCount += 1;
      },
    });
    host.accept(
      executeV4(
        'inv_v4_process_invalid',
        `
          const requests = [
            [],
            [{ executable: 'bun', unknown: true }],
            [{ executable: 'sh -c' }],
            [{ executable: 'bun', args: [() => null] }],
            [{ executable: 'bun', cwd: '../outside' }],
            [{ executable: 'bun', env: { 'BAD-NAME': 'value' } }],
            [{ executable: 'bun', timeout: '25h' }],
            [{ executable: 'bun', acceptExitCodes: [0, 0] }],
            [{ executable: 'bun', output: { maxBytes: 262145 } }],
            [{ executable: 'bun', env: new Map([['CI', 'true']]) }]
          ];
          const errors = [];
          for (const args of requests) {
            try {
              await services.process.run(...args);
              errors.push('accepted');
            } catch (error) {
              errors.push(error.message);
            }
          }
          return errors;
        `
      )
    );
    await host.drain();

    expect(callCount).toBe(0);
    expect(completed[0]?.outcome.kind).toBe('success');
    if (completed[0]?.outcome.kind !== 'success') {
      throw new Error('Expected local Process validation to be catchable.');
    }
    const errors = completed[0].outcome.value as string[];
    expect(errors).toHaveLength(10);
    expect(errors).not.toContain('accepted');
    expect(errors.join('\n')).toContain('runtime-approved alias');
    expect(errors.join('\n')).toContain('relative project directory');
    expect(errors.join('\n')).toContain('unique integers');
  });

  test('Process v1 requires stable names for multiple calls in one step', async () => {
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
        const processResult = {
          contract: 'woml.process',
          contractVersion: 1,
          kind: 'result',
          exitCode: 0,
          signal: null,
          stdout: '',
          stderr: '',
          stdoutBytes: 0,
          stderrBytes: 0,
          stdoutTruncated: false,
          stderrTruncated: false,
          durationMs: 1,
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
            resultBytes: Buffer.byteLength(JSON.stringify(processResult)),
            durationMs: 1,
            result: processResult,
          },
        });
      },
    });
    host.accept(
      executeV4(
        'inv_v4_process_names',
        `
          await services.process.run({ executable: 'bun' });
          try {
            await services.process.run({ executable: 'bun' });
            return { rejected: false };
          } catch (error) {
            return { rejected: true, message: error.message };
          }
        `
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

  test('Locks v1 exposes a frozen callback lease and always releases it', async () => {
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
        const result =
          message.call.operation === 'acquire'
            ? {
                contract: 'woml.locks-capability',
                contractVersion: 1,
                kind: 'acquired',
                sessionId: 'lock_session_1',
                key: 'inventory/customer-42',
                fencingToken: '7',
                acquiredAt: '2026-10-20T10:00:00.000Z',
              }
            : message.call.operation === 'assert-current'
              ? {
                  contract: 'woml.locks-capability',
                  contractVersion: 1,
                  kind: 'current',
                  key: 'inventory/customer-42',
                }
              : {
                  contract: 'woml.locks-capability',
                  contractVersion: 1,
                  kind: 'released',
                  key: 'inventory/customer-42',
                };
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
            resultBytes: Buffer.byteLength(JSON.stringify(result)),
            durationMs: 1,
            result,
          },
        });
      },
    });

    host.accept(
      executeV4(
        'inv_v4_locks',
        `
          const facadeFrozen = Object.isFrozen(services.locks);
          const methodFrozen = Object.isFrozen(services.locks.withLease);
          const value = await services.locks.withLease(
            'inventory/customer-42',
            { acquireTimeout: '5s', lease: '5s', name: 'reserve-customer' },
            async lease => {
              await lease.assertCurrent();
              return {
                leaseFrozen: Object.isFrozen(lease),
                assertionFrozen: Object.isFrozen(lease.assertCurrent),
                key: lease.key,
                fencingToken: lease.fencingToken,
                acquiredAt: lease.acquiredAt
              };
            }
          );
          return { facadeFrozen, methodFrozen, value };
        `
      )
    );
    await host.drain();

    expect(calls.map(call => call.call.operation)).toEqual([
      'acquire',
      'assert-current',
      'release',
    ]);
    expect(calls[0]?.call).toMatchObject({
      capability: 'locks',
      identity: {
        mode: 'named',
        operationName: 'locks.acquire.reserve-customer',
      },
      input: {
        key: 'inventory/customer-42',
        acquireTimeoutMs: 5_000,
        leaseMs: 5_000,
        operationIdentity: 'locks.acquire.reserve-customer',
      },
    });
    expect(calls[1]?.call.input).toEqual({ sessionId: 'lock_session_1' });
    expect(calls[2]?.call.input).toEqual({ sessionId: 'lock_session_1' });
    expect(completed[0]?.outcome).toEqual({
      kind: 'success',
      value: {
        facadeFrozen: true,
        methodFrozen: true,
        value: {
          leaseFrozen: true,
          assertionFrozen: true,
          key: 'inventory/customer-42',
          fencingToken: '7',
          acquiredAt: '2026-10-20T10:00:00.000Z',
        },
      },
    });
  });

  test('Locks v1 releases before propagating a callback failure', async () => {
    const completed: CompletedMessage[] = [];
    const operations: string[] = [];
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
        operations.push(message.call.operation);
        const result =
          message.call.operation === 'acquire'
            ? {
                contract: 'woml.locks-capability',
                contractVersion: 1,
                kind: 'acquired',
                sessionId: 'lock_session_failure',
                key: 'deploy/api',
                fencingToken: '8',
                acquiredAt: '2026-10-20T10:00:00.000Z',
              }
            : {
                contract: 'woml.locks-capability',
                contractVersion: 1,
                kind: 'released',
                key: 'deploy/api',
              };
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
            resultBytes: Buffer.byteLength(JSON.stringify(result)),
            durationMs: 1,
            result,
          },
        });
      },
    });

    host.accept(
      executeV4(
        'inv_v4_locks_failure',
        `
          try {
            await services.locks.withLease(
              'deploy/api',
              { acquireTimeout: '5s', lease: '5s', name: 'deploy-api' },
              async () => { throw new Error('deployment failed'); }
            );
            return { caught: false };
          } catch (error) {
            return { caught: true, message: error.message };
          }
        `
      )
    );
    await host.drain();

    expect(operations).toEqual(['acquire', 'release']);
    expect(completed[0]?.outcome).toEqual({
      kind: 'success',
      value: { caught: true, message: 'deployment failed' },
    });
  });

  test('Artifact v1 normalizes calls and exposes immutable bounded public values', async () => {
    const completed: CompletedMessage[] = [];
    const calls: CapabilityCallMessage[] = [];
    const reference = {
      contract: 'woml.artifact-reference',
      contractVersion: 1,
      id: 'artifact_01k7qh7s00a1b2c3d4e5f6g7h8',
      name: 'Terraform plan 🌍',
      size: 184_320,
      sha256: 'c742b8669e07a0ca8a2a8d79b22b17504c51f7b0d81623a8045fece436f02942',
      contentType: 'application/octet-stream',
    } as const;
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
        const operation = message.call.operation as 'put' | 'materialize';
        const artifactResult = {
          contract: 'woml.artifacts',
          contractVersion: 1,
          kind: 'result',
          operation,
          data:
            operation === 'put'
              ? reference
              : { artifact: reference, path: '/private/attempt/release.tfplan' },
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
            resultBytes: Buffer.byteLength(JSON.stringify(artifactResult)),
            durationMs: 4,
            result: artifactResult,
          },
        });
      },
    });

    host.accept(
      executeV4(
        'inv_v4_artifacts',
        `
          const facadeFrozen = Object.isFrozen(services.artifacts);
          const reference = await services.artifacts.put(
            'infrastructure/release.tfplan',
            { name: 'Terraform plan 🌍', retention: '30d' }
          );
          const path = await services.artifacts.materialize(reference);
          return {
            facadeFrozen,
            referenceFrozen: Object.isFrozen(reference),
            reference,
            pathAvailable: path.endsWith('release.tfplan')
          };
        `
      )
    );
    await host.drain();

    expect(calls).toHaveLength(2);
    expect(calls[0]?.call).toMatchObject({
      capability: 'artifacts',
      operation: 'put',
      identity: { mode: 'named', operationName: 'Terraform plan 🌍' },
      limits: { resultBytes: 16_384 },
      input: {
        contract: 'woml.artifacts',
        contractVersion: 1,
        kind: 'request',
        operation: 'put',
        input: {
          sourcePath: 'infrastructure/release.tfplan',
          name: 'Terraform plan 🌍',
          contentType: 'application/octet-stream',
          retentionMs: 2_592_000_000,
        },
      },
    });
    expect(calls[0]?.call.identity.operationKey).toMatch(/^sha256:[a-f0-9]{64}$/);
    expect(calls[1]?.call).toMatchObject({
      capability: 'artifacts',
      operation: 'materialize',
      identity: { mode: 'automatic' },
      input: {
        operation: 'materialize',
        input: { artifact: reference },
      },
    });
    expect(completed[0]?.outcome).toEqual({
      kind: 'success',
      value: {
        facadeFrozen: true,
        referenceFrozen: true,
        reference,
        pathAvailable: true,
      },
    });
  });

  test('Artifact v1 prevents private materialization paths from becoming durable output', async () => {
    const completed: CompletedMessage[] = [];
    const reference = {
      contract: 'woml.artifact-reference',
      contractVersion: 1,
      id: 'artifact_01k7qh7s00a1b2c3d4e5f6g7h8',
      name: 'release-plan',
      size: 12,
      sha256: 'c742b8669e07a0ca8a2a8d79b22b17504c51f7b0d81623a8045fece436f02942',
      contentType: 'application/octet-stream',
    } as const;
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
        const result = {
          contract: 'woml.artifacts',
          contractVersion: 1,
          kind: 'result',
          operation: 'materialize',
          data: {
            artifact: reference,
            path: '/private/attempt/release.tfplan',
          },
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
            resultBytes: Buffer.byteLength(JSON.stringify(result)),
            durationMs: 1,
            result,
          },
        });
      },
    });
    host.accept(
      executeV4(
        'inv_v4_artifact_path_leak',
        `
          const path = await services.artifacts.materialize(${JSON.stringify(reference)});
          return { path };
        `
      )
    );
    await host.drain();
    expect(completed[0]?.outcome).toMatchObject({
      kind: 'failure',
      error: {
        message: expect.stringContaining('attempt-scoped'),
      },
    });
    expect(JSON.stringify(completed[0])).not.toContain(
      '/private/attempt/release.tfplan'
    );
  });

  test('Artifact v1 rejects unsafe requests before crossing the capability boundary', async () => {
    const completed: CompletedMessage[] = [];
    let callCount = 0;
    const host = new ScriptHost({
      workerUrl: new URL('../src/script-host-worker.ts', import.meta.url),
      protocolVersion: 4,
      send: async message => {
        if (message.messageType === 'completed') completed.push(message);
        if (message.messageType === 'capability_call') callCount += 1;
      },
    });
    host.accept(
      executeV4(
        'inv_v4_artifacts_invalid',
        `
          const requests = [
            () => services.artifacts.put('../secret', { name: 'bad' }),
            () => services.artifacts.put('report.txt', {}),
            () => services.artifacts.put('report.txt', { name: 'bad', retention: '1.5d' }),
            () => services.artifacts.put('report.txt', { name: 'bad', contentType: 'not mime' }),
            () => services.artifacts.head({ contract: 'woml.artifact-reference', contractVersion: 2 })
          ];
          const errors = [];
          for (const invoke of requests) {
            try {
              await invoke();
              errors.push('accepted');
            } catch (error) {
              errors.push(error.message);
            }
          }
          return errors;
        `
      )
    );
    await host.drain();

    expect(callCount).toBe(0);
    expect(completed[0]?.outcome.kind).toBe('success');
    if (completed[0]?.outcome.kind !== 'success') {
      throw new Error('Expected local Artifact validation to be catchable.');
    }
    expect(completed[0].outcome.value).not.toContain('accepted');
  });
});

describe('Script Host v5 module runtime', () => {
  test('keeps an explicitly imported services.telegram module ahead of the built-in capability', async () => {
    const bundle = `export async function send(value) {
      return { source: 'local-module', value };
    }`;
    const digest = moduleDigest(bundle);
    const request = executeV5(
      'inv_module_telegram_shadow',
      `return services.telegram.send('local behavior');`,
      digest,
      ['send']
    );
    const result = await runHost(
      [
        {
          protocol: 'woml.script-host',
          protocolVersion: 5,
          messageType: 'register_module',
          bundleDigest: digest,
          bundle,
        },
        {
          ...request,
          modules: [{ name: 'telegram', bundleDigest: digest, exports: ['send'] }],
        },
      ],
      { WOML_SCRIPT_HOST_PROTOCOL_VERSION: '5' }
    );

    expect(result.exitCode).toBe(0);
    expect(
      byInvocation(result.messages).get('inv_module_telegram_shadow')
    ).toMatchObject({
      outcome: {
        kind: 'success',
        value: { source: 'local-module', value: 'local behavior' },
      },
    });
    expect(
      result.messages.some(message => message.messageType === 'capability_call')
    ).toBe(false);
  });

  test('loads a large verified module without data-URL filename limits', async () => {
    const bundle = `export function value() { return "large-bundle-ok"; }\n${'// bounded module padding\n'.repeat(4096)}`;
    const digest = moduleDigest(bundle);
    const result = await runHost([
      { protocol: 'woml.script-host', protocolVersion: 5, messageType: 'register_module', bundleDigest: digest, bundle },
      executeV5('inv_large_module', 'return services.utility.value();', digest, ['value']),
    ], { WOML_SCRIPT_HOST_PROTOCOL_VERSION: '5' });
    expect(result.exitCode).toBe(0);
    expect(byInvocation(result.messages).get('inv_large_module')).toMatchObject({
      outcome: { kind: 'success', value: 'large-bundle-ok' },
    });
  });

  test('routes imported module console output away from protocol stdout and redacts secrets', async () => {
    const bundle = `export function report(value) { console.log('module log', value); console.warn(value); return true; }`;
    const digest = moduleDigest(bundle);
    const request = executeV5('inv_module_console', 'return services.utility.report(secrets.API_TOKEN);', digest, ['report']);
    const result = await runHost([
      { protocol: 'woml.script-host', protocolVersion: 5, messageType: 'register_module', bundleDigest: digest, bundle },
      { ...request, bindings: { bindingVersion: 1, servicesVersion: 1, secrets: { API_TOKEN: 'module-secret-value' } } },
    ], { WOML_SCRIPT_HOST_PROTOCOL_VERSION: '5' });
    expect(result.exitCode).toBe(0);
    expect(byInvocation(result.messages).get(request.invocationId)?.outcome).toEqual({ kind: 'success', value: true });
    expect(result.stderr).toContain('module log');
    expect(result.stderr).not.toContain('module-secret-value');
  });

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

describe('Script Host v6 recoverability and diagnostics', () => {
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

describe('Script Host v7 lifecycle mode', () => {
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

describe('Reusable custom-step Script Host v8', () => {
  test('injects deeply read-only props and keeps invocation state isolated', async () => {
    const first = executeReusableV8(
      'inv_reusable_first',
      `
        globalThis.counter = (globalThis.counter ?? 0) + 1;
        try { props.price = 999; } catch {}
        return { total: props.price * props.quantity, counter: globalThis.counter };
      `,
      { price: 12, quantity: 3 }
    );
    const result = await runHost(
      [first, { ...first, invocationId: 'inv_reusable_second' }],
      { WOML_SCRIPT_HOST_PROTOCOL_VERSION: '8' }
    );
    const completed = result.messages.filter(
      message => message.messageType === 'completed'
    ) as CompletedMessage[];
    expect(completed.map(message => message.outcome)).toEqual([
      { kind: 'success', value: { total: 36, counter: 1 } },
      { kind: 'success', value: { total: 36, counter: 1 } },
    ]);
  });

  test('injects reusable lifecycle state and prevents secret props from becoming results', async () => {
    const lifecycle: ExecuteMessageV8 = {
      ...executeReusableV8(
        'inv_reusable_lifecycle',
        `
          if (!Object.isFrozen(props) || !Object.isFrozen(lifecycle)) throw new Error('mutable');
          if (lifecycle.outcome !== 'succeeded' || lifecycle.result.total !== 36) throw new Error('binding');
        `,
        { price: 12, quantity: 3 }
      ),
      mode: 'lifecycle',
      handler: 'runtime.lifecycle-script',
      nodeId: 'discount:on-success:0',
      reusableLifecycle: {
        hook: 'on-success',
        outcome: 'succeeded',
        result: { total: 36 },
      },
    };
    const secret = 'secret-v8-value';
    const leaking = executeReusableV8(
      'inv_reusable_secret',
      `return { token: props.token };`,
      { token: secret },
      { API_TOKEN: secret }
    );
    const result = await runHost([lifecycle, leaking], {
      WOML_SCRIPT_HOST_PROTOCOL_VERSION: '8',
    });
    const completed = result.messages.filter(
      message => message.messageType === 'completed'
    ) as CompletedMessage[];
    const outcomes = byInvocation(completed);
    expect(outcomes.get('inv_reusable_lifecycle')?.outcome).toEqual({
      kind: 'success', value: null,
    });
    expect(outcomes.get('inv_reusable_secret')?.outcome).toMatchObject({
      kind: 'failure',
      error: { kind: 'invalid_script_result' },
    });
    expect(JSON.stringify(result.messages)).not.toContain(secret);
  });
});

describe('Durable Lock Script Host v10', () => {
  test('exposes a deeply read-only durable lock binding', async () => {
    const message: ExecuteMessageV10 = {
      ...executeStepV7(
        'inv_lock_binding',
        `
          let mutationBlocked = false;
          try { context.lock.key = 'changed'; } catch { mutationBlocked = true; }
          return {
            id: context.lock.id,
            key: context.lock.key,
            fencingToken: context.lock.fencingToken,
            acquiredAt: context.lock.acquiredAt,
            frozen: Object.isFrozen(context.lock),
            mutationBlocked,
          };
        `
      ),
      protocolVersion: 10,
      context: {
        trigger: {},
        steps: {},
        lock: {
          id: 'deployment',
          key: 'production/payments',
          fencingToken: '42',
          acquiredAt: '2026-10-20T12:00:00.000Z',
        },
      },
    };
    const result = await runHost([message], {
      WOML_SCRIPT_HOST_PROTOCOL_VERSION: '10',
    });
    expect(
      byInvocation(result.messages).get(message.invocationId)?.outcome
    ).toEqual({
      kind: 'success',
      value: {
        id: 'deployment',
        key: 'production/payments',
        fencingToken: '42',
        acquiredAt: '2026-10-20T12:00:00.000Z',
        frozen: true,
        mutationBlocked: true,
      },
    });
    expect(
      isScriptHostMessage({
        ...message,
        context: {
          ...message.context,
          lock: { ...message.context.lock, fencingToken: '0' },
        },
      })
    ).toBe(false);
  });
});

describe('Structured progress and script logs in Script Host v12', () => {
  test('awaits progress acknowledgements and emits redacted structured console levels', async () => {
    const outbound: ScriptHostMessage[] = [];
    let host!: ScriptHost;
    host = new ScriptHost({
      workerUrl: new URL('../src/script-host-worker.ts', import.meta.url),
      protocolVersion: 12,
      send: async message => {
        outbound.push(message);
        if (message.messageType === 'progress_report') {
          host.accept({
            protocol: 'woml.script-host',
            protocolVersion: 12,
            messageType: 'progress_ack',
            requestId: message.requestId,
            frameSequence: message.frameSequence,
            accepted: true,
          });
        }
      },
    });
    const request: ExecuteMessageV12 = {
      ...executeStepV7(
        'inv_progress_v12',
        `
          console.debug('starting', secrets.token);
          await services.progress.report({
            id: 'files', current: 1, total: 2, unit: 'files',
            message: \`processed \${secrets.token}\`
          });
          await services.progress.report({ id: 'waiting', message: 'Waiting for input' });
          return {
            progressFrozen: Object.isFrozen(services.progress),
            reportFrozen: Object.isFrozen(services.progress.report)
          };
        `
      ),
      protocolVersion: 12,
      bindings: {
        bindingVersion: 1,
        servicesVersion: 1,
        secrets: { token: 'never-persist-this-token' },
      },
    };

    host.accept(request);
    await host.drain();

    expect(outbound.filter(message => message.messageType === 'script_log')).toEqual([
      expect.objectContaining({
        protocolVersion: 12,
        requestId: request.invocationId,
        frameSequence: 1,
        level: 'debug',
        text: 'starting [REDACTED]',
      }),
    ]);
    expect(outbound.filter(message => message.messageType === 'progress_report')).toEqual([
      expect.objectContaining({
        requestId: request.invocationId,
        frameSequence: 2,
        progressId: 'files',
        current: 1,
        total: 2,
        unit: 'files',
        message: 'processed [REDACTED]',
      }),
      expect.objectContaining({
        requestId: request.invocationId,
        frameSequence: 3,
        progressId: 'waiting',
        message: 'Waiting for input',
      }),
    ]);
    expect(byInvocation(outbound).get(request.invocationId)?.outcome).toEqual({
      kind: 'success',
      value: { progressFrozen: true, reportFrozen: true },
    });
  });

  test('gives imported modules the same deeply frozen progress service', async () => {
    const outbound: ScriptHostMessage[] = [];
    let host!: ScriptHost;
    host = new ScriptHost({
      workerUrl: new URL('../src/script-host-worker.ts', import.meta.url),
      protocolVersion: 12,
      send: async message => {
        outbound.push(message);
        if (message.messageType === 'progress_report') {
          host.accept({
            protocol: 'woml.script-host',
            protocolVersion: 12,
            messageType: 'progress_ack',
            requestId: message.requestId,
            frameSequence: message.frameSequence,
            accepted: true,
          });
        }
      },
    });
    const bundle = `export async function report() {
      await services.progress.report({ id: 'module', current: 1, total: 1 });
      return Object.isFrozen(services.progress) && Object.isFrozen(services.progress.report);
    }`;
    const sourceMap = JSON.stringify({
      version: 3,
      sources: ['modules/progress.ts'],
      sourcesContent: [bundle],
      names: [],
      mappings: '',
    });
    const bundleDigest = moduleDigest(bundle);
    host.accept({
      protocol: 'woml.script-host',
      protocolVersion: 12,
      messageType: 'register_module',
      bundleDigest,
      bundle,
      sourceMapDigest: moduleDigest(sourceMap),
      sourceMap,
    });
    const request: ExecuteMessageV12 = {
      ...executeStepV7(
        'inv_module_progress_v12',
        'return { moduleProgressFrozen: await services.utility.report() };'
      ),
      protocolVersion: 12,
      modules: [{ name: 'utility', bundleDigest, exports: ['report'] }],
    };
    host.accept(request);
    await host.drain();

    expect(outbound).toContainEqual(
      expect.objectContaining({
        messageType: 'progress_report',
        requestId: request.invocationId,
        progressId: 'module',
        current: 1,
        total: 1,
      })
    );
    expect(byInvocation(outbound).get(request.invocationId)?.outcome).toEqual({
      kind: 'success',
      value: { moduleProgressFrozen: true },
    });
  });

  test('rejects invalid v12 progress and preserves v11 console behavior', async () => {
    expect(
      isScriptHostMessage({
        protocol: 'woml.script-host',
        protocolVersion: 12,
        messageType: 'progress_report',
        requestId: 'inv_invalid_progress',
        frameSequence: 1,
        progressId: 'files',
        current: Number.MAX_SAFE_INTEGER + 1,
      })
    ).toBe(false);

    const legacy: ExecuteMessageV11 = {
      ...executeStepV7('inv_v11_console_compatibility', `console.log('legacy-v11'); return true;`),
      protocolVersion: 11,
    };
    const result = await runHost([legacy], {
      WOML_SCRIPT_HOST_PROTOCOL_VERSION: '11',
    });
    expect(result.messages.some(message => message.messageType === 'script_log')).toBe(false);
    expect(result.stderr).toContain('legacy-v11');
  });
});

describe('Correction Script Host v11', () => {
  test('exposes exactly one deeply read-only correction binding', async () => {
    const compensation: ExecuteMessageV11 = {
      ...executeStepV7(
        'inv_compensation_binding',
        `
          let mutationBlocked = false;
          try { context.compensation.reason = 'cancel'; } catch { mutationBlocked = true; }
          return {
            binding: context.compensation,
            cleanupAbsent: context.cleanup === undefined,
            frozen: Object.isFrozen(context.compensation),
            mutationBlocked,
          };
        `
      ),
      protocolVersion: 11,
      context: {
        trigger: {},
        steps: {},
        compensation: {
          sagaId: 'provision',
          forStepId: 'createNetwork',
          reason: 'failure',
        },
      },
    };
    const cleanup: ExecuteMessageV11 = {
      ...executeStepV7(
        'inv_cleanup_binding',
        `return {
          binding: context.cleanup,
          compensationAbsent: context.compensation === undefined,
          frozen: Object.isFrozen(context.cleanup),
        };`
      ),
      protocolVersion: 11,
      context: {
        trigger: {},
        steps: {},
        cleanup: {
          sagaId: 'provision',
          reason: 'failure',
          compensationStatus: 'completed',
        },
      },
    };
    const result = await runHost([compensation, cleanup], {
      WOML_SCRIPT_HOST_PROTOCOL_VERSION: '11',
    });
    expect(byInvocation(result.messages).get(compensation.invocationId)?.outcome).toEqual({
      kind: 'success',
      value: {
        binding: {
          sagaId: 'provision',
          forStepId: 'createNetwork',
          reason: 'failure',
        },
        cleanupAbsent: true,
        frozen: true,
        mutationBlocked: true,
      },
    });
    expect(byInvocation(result.messages).get(cleanup.invocationId)?.outcome).toEqual({
      kind: 'success',
      value: {
        binding: {
          sagaId: 'provision',
          reason: 'failure',
          compensationStatus: 'completed',
        },
        compensationAbsent: true,
        frozen: true,
      },
    });
    expect(
      isScriptHostMessage({
        ...compensation,
        context: {
          ...compensation.context,
          cleanup: cleanup.context.cleanup,
        },
      })
    ).toBe(false);
  });
});

describe('For-each Script Host v9', () => {
  test('exposes deeply read-only item and stable iteration bindings', async () => {
    const message: ExecuteMessageV9 = {
      ...executeStepV7(
        'inv_for_each_binding',
        `
          let mutationBlocked = false;
          try { context.item.name = 'changed'; } catch { mutationBlocked = true; }
          try { context.iteration.index = 99; } catch { mutationBlocked = true; }
          return {
            name: context.item.name,
            index: context.iteration.index,
            total: context.iteration.total,
            frozen: Object.isFrozen(context.item) && Object.isFrozen(context.iteration),
            mutationBlocked,
          };
        `
      ),
      protocolVersion: 9,
      context: {
        trigger: {},
        steps: {},
        item: { name: 'alpha' },
        iteration: { index: 0, total: 3 },
      },
    };
    const result = await runHost([message], {
      WOML_SCRIPT_HOST_PROTOCOL_VERSION: '9',
    });
    expect(
      byInvocation(result.messages).get(message.invocationId)?.outcome
    ).toEqual({
      kind: 'success',
      value: {
        name: 'alpha',
        index: 0,
        total: 3,
        frozen: true,
        mutationBlocked: true,
      },
    });
  });

  test('rejects partial and out-of-range iteration envelopes', () => {
    const valid: ExecuteMessageV9 = {
      ...executeStepV7('inv_for_each_invalid', 'return null;'),
      protocolVersion: 9,
      context: {
        trigger: {},
        steps: {},
        item: null,
        iteration: { index: 0, total: 1 },
      },
    };
    expect(isScriptHostMessage(valid)).toBe(true);
    expect(
      isScriptHostMessage({ ...valid, context: { trigger: {}, steps: {}, item: null } })
    ).toBe(false);
    expect(
      isScriptHostMessage({
        ...valid,
        context: {
          trigger: {}, steps: {}, item: null, iteration: { index: 1, total: 1 },
        },
      })
    ).toBe(false);
  });
});

describe('Script Host v7 durable user state', () => {
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
