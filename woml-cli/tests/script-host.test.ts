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
  CompletedMessage,
  ExecuteMessage,
  ScriptHostMessage,
} from '../src/script-host/types';

const packageRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(packageRoot, '..');
const hostEntry =
  process.env.WOML_SCRIPT_HOST_TEST_ENTRY ??
  resolve(packageRoot, 'src/script-host.ts');
const bunExecutable = Bun.which('bun')!;

function execute(
  invocationId: string,
  source: string,
  options: {
    readonly context?: ExecuteMessage['context'];
    readonly timeoutMs?: number;
  } = {},
): ExecuteMessage {
  return {
    protocol: 'woml.script-host',
    protocolVersion: 1,
    messageType: 'execute',
    invocationId,
    runId: 'run_host_test_01',
    nodeId: invocationId.replace(/^inv_/, ''),
    attempt: 1,
    handler: 'runtime.script',
    timeoutMs: options.timeoutMs ?? 1000,
    source,
    context: options.context ?? { trigger: {}, steps: {} },
  };
}

interface HostRunResult {
  readonly messages: ScriptHostMessage[];
  readonly stderr: string;
  readonly exitCode: number;
}

async function runHost(
  messages: readonly unknown[],
  environment: Readonly<Record<string, string>> = {},
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
  for (const message of decoded) expect(isScriptHostMessage(message)).toBe(true);

  return {
    messages: decoded as ScriptHostMessage[],
    stderr,
    exitCode,
  };
}

function completions(messages: readonly ScriptHostMessage[]): CompletedMessage[] {
  return messages.filter(
    (message): message is CompletedMessage => message.messageType === 'completed',
  );
}

function byInvocation(
  messages: readonly ScriptHostMessage[],
): ReadonlyMap<string, CompletedMessage> {
  return new Map(
    completions(messages).map((message) => [message.invocationId, message]),
  );
}

describe('production Content-Length framing', () => {
  test('decodes multibyte and CRLF content one byte at a time', async () => {
    const fixture = await Bun.file(
      resolve(
        packageRoot,
        'tests/fixtures/script-host/unicode-crlf.execute.v1.json',
      ),
    ).json();
    const frame = encodeFrame(fixture);
    const body = JSON.stringify(fixture);
    expect(new TextEncoder().encode(body).byteLength).toBeGreaterThan(body.length);

    const decoder = new FrameDecoder();
    const messages: unknown[] = [];
    for (const byte of frame) messages.push(...decoder.push(Uint8Array.of(byte)));
    decoder.finish();

    expect(messages).toEqual([fixture]);
  });

  test('decodes combined frames and rejects malformed, oversized, or incomplete input', () => {
    const first = { id: 1 };
    const second = { id: 2 };
    const decoder = new FrameDecoder();
    expect(
      decoder.push(Buffer.concat([encodeFrame(first), encodeFrame(second)])),
    ).toEqual([first, second]);
    decoder.finish();

    expect(() =>
      new FrameDecoder().push(Buffer.from('Content-Length: nope\r\n\r\n', 'ascii')),
    ).toThrow(FrameProtocolError);
    expect(() =>
      new FrameDecoder({ maxFrameBytes: 10 }).push(
        Buffer.from('Content-Length: 11\r\n\r\n', 'ascii'),
      ),
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
        ]),
      ),
    ).toThrow(FrameProtocolError);
  });
});

describe('long-lived Bun script host', () => {
  test('provides the frozen context contract with top-level await', async () => {
    const result = await runHost([
      execute(
        'inv_context_contract',
        `await Promise.resolve();
return {
  greeting: \`Hello \${context.trigger.name}\`,
  hasRun: "run" in context,
  hasServices: typeof services !== "undefined"
};`,
        { context: { trigger: { name: 'Ada' }, steps: {} } },
      ),
      execute(
        'inv_frozen_context',
        'context.trigger.name = "Changed"; return { unreachable: true };',
        { context: { trigger: { name: 'Original' }, steps: {} } },
      ),
    ]);
    const indexed = byInvocation(result.messages);

    expect(indexed.get('inv_context_contract')?.outcome).toEqual({
      kind: 'success',
      value: {
        greeting: 'Hello Ada',
        hasRun: false,
        hasServices: false,
      },
    });
    expect(indexed.get('inv_frozen_context')?.outcome).toMatchObject({
      kind: 'failure',
      error: { kind: 'script_threw', code: 'WOML_SCRIPT_THROWN' },
    });
  });

  test('multiplexes invocations and correlates out-of-order responses', async () => {
    const result = await runHost([
      execute(
        'inv_slow',
        'await new Promise((resolve) => setTimeout(resolve, 180)); return { order: "slow" };',
      ),
      execute(
        'inv_fast',
        'await new Promise((resolve) => setTimeout(resolve, 5)); return { order: "fast" };',
      ),
      execute(
        'inv_middle',
        'await new Promise((resolve) => setTimeout(resolve, 80)); return { order: "middle" };',
      ),
    ]);

    expect(result.exitCode).toBe(0);
    expect(result.stderr).toBe('');
    expect(result.messages[0].messageType).toBe('ready');
    expect(completions(result.messages).map((message) => message.invocationId)).toEqual([
      'inv_fast',
      'inv_middle',
      'inv_slow',
    ]);
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

  test('uses fresh Worker globals and does not pass host environment secrets', async () => {
    const result = await runHost(
      [
        execute(
          'inv_set_global',
          'globalThis.__womlLeak = "secret-state"; return { set: true };',
        ),
        execute(
          'inv_read_global',
          'await new Promise((resolve) => setTimeout(resolve, 40)); return { leaked: globalThis.__womlLeak ?? null, env: process.env.WOML_TEST_SECRET ?? null };',
        ),
      ],
      { WOML_TEST_SECRET: 'must-not-enter-worker' },
    );

    expect(result.exitCode).toBe(0);
    expect(byInvocation(result.messages).get('inv_read_global')?.outcome).toEqual({
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
        'const value = {}; value.self = value; return value;',
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

  test('reports a Worker startup crash separately', async () => {
    const sent: CompletedMessage[] = [];
    const host = new ScriptHost({
      workerUrl: new URL('./fixtures/missing-worker.ts', import.meta.url),
      send: async (message) => {
        sent.push(message);
      },
    });

    host.accept(execute('inv_worker_crash', 'return { unreachable: true };'));
    await host.drain();

    expect(sent).toHaveLength(1);
    expect(sent[0].outcome).toMatchObject({
      kind: 'failure',
      error: { kind: 'worker_crashed', code: 'WOML_SCRIPT_WORKER_CRASHED' },
    });
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
        execute(
          'inv_result_large',
          'return { payload: "x".repeat(300) };',
        ),
      ],
      {
        WOML_SCRIPT_HOST_MAX_CONTEXT_BYTES: '200',
        WOML_SCRIPT_HOST_MAX_RESULT_BYTES: '200',
      },
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
      protocolVersion: 2,
    };
    const result = await runHost([invalid]);

    expect(result.exitCode).toBe(1);
    expect(result.messages).toHaveLength(1);
    expect(result.messages[0].messageType).toBe('ready');
    expect(result.stderr).toContain('MessageProtocolError');
    expect(result.stderr).not.toContain('return { ok: true }');
  });
});
