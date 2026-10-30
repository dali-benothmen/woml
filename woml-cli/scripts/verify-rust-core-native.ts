#!/usr/bin/env bun
// RCM13: load real addons, not a Rust mock or a source-only export inventory.
import assert from 'node:assert/strict';
import { copyFile, mkdtemp, readdir, rm } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { compileWoml, parseWoml } from '@woml/compiler';
import { compiledDefinitionHash } from '../src/rust-executor';
import { defaultStagedNativePath, expectedNativeExports } from './verify-native-exports';

type Native = Record<string, (...args: unknown[]) => unknown>;
const root = resolve(import.meta.dir, '../..');

function json(value: unknown): Record<string, unknown> {
  assert.equal(typeof value, 'string');
  return JSON.parse(value as string) as Record<string, unknown>;
}

function promise(value: unknown): Promise<unknown> {
  assert.ok(value instanceof Promise, 'native async/callback API must return a Promise');
  return value;
}

async function eventually(check: () => boolean, label: string): Promise<void> {
  const end = performance.now() + 10_000;
  while (!check()) {
    assert.ok(performance.now() < end, `native ${label} did not settle`);
    await Bun.sleep(10);
  }
}

export async function verifyRustCoreNative(source: string) {
  const directory = await mkdtemp(join(tmpdir(), 'woml-rcm13-addon-'));
  const addon = join(directory, 'candidate.node');
  let native: Native | undefined;
  let runtimeId: string | undefined;
  try {
    await copyFile(resolve(source), addon);
    native = createRequire(import.meta.url)(addon) as Native;
    const names = Object.keys(native).sort();
    assert.deepEqual(names, [...expectedNativeExports].sort());
    const surface = Object.fromEntries(names.map(name => {
      assert.equal(typeof native![name], 'function', name);
      return [name, { type: typeof native![name], length: native![name]!.length }];
    }));
    const host = resolve(root, 'woml-cli/src/script-host.ts');
    const executable = Bun.which('bun')!;
    const legacy = await Bun.file(resolve(root, 'woml/tests/fixtures/hello.compiled.v1.json')).text();
    const hash = `sha256:${'13'.repeat(32)}`;
    const ordinary = json(await promise(native.executeWomlWorkflow!(
      legacy, hash, '{}', executable, host, 5_000, '{}', '[]', undefined,
    )));
    assert.ok(JSON.stringify(ordinary).includes('Hello World'));
    // Async decode failures reject; synchronous inspection failures throw a
    // stable structured envelope. Callback-API decode errors remain synchronous.
    await assert.rejects(promise(native.executeWomlWorkflow!(
      '{', hash, '{}', executable, host, 5_000, '{}', '[]', undefined,
    )), /Invalid compiled workflow JSON/);
    const state = join(directory, 'durable.sqlite');
    let inspectionCode: unknown;
    assert.throws(() => native!.inspectWomlRun!(state, 'run_missing'), (error: unknown) => {
      inspectionCode = json((error as Error).message).code;
      return inspectionCode === 'WOML_RUN_NOT_FOUND';
    });
    assert.throws(() => native!.executeWomlWorkflowDurableWithProgress!(
      '{', hash, '{}', executable, host, 5_000, state, () => {}, '{}', '[]', undefined,
    ), /Invalid compiled workflow JSON/);

    const workflow = compileWoml(parseWoml(`<woml>
      <workflow id="rcm13-contract" version="1.0.0">
        <lifecycle><on-start><script>console.log("RCM13 lifecycle");</script></on-start></lifecycle>
        <triggers><manual id="start" /></triggers>
        <steps><step id="hello"><script>console.log("RCM13 output"); return { message: "native bridge unchanged" };</script></step></steps>
      </workflow></woml>`, { file: 'rcm13-contract.woml' }));
    const encoded = JSON.stringify(workflow);
    const definitionHash = compiledDefinitionHash(workflow);
    const progress: string[] = [];
    const result = json(await promise(native.executeWomlWorkflowDurableWithProgress!(
      encoded, definitionHash, '{}', executable, host, 5_000, state,
      (value: string) => { progress.push(value); }, '{}', '[]', undefined,
    )));
    assert.equal(typeof result.runId, 'string');
    await eventually(() => progress.length > 0, 'progress callback');
    progress.forEach(value => json(value));
    await eventually(() => progress.some(value => value.includes('woml.lifecycle-progress/v1')), 'lifecycle callback');
    await eventually(() => progress.some(value => value.includes('woml.operational-output-wakeup/v1')), 'output wakeup');
    const inspection = json(native.inspectWomlRun!(state, result.runId));
    assert.equal(inspection.status, 'succeeded');
    assert.deepEqual(inspection.result, { message: 'native bridge unchanged' });
    assert.equal(native.hasWomlWorkflowDefinition!(state, workflow.workflowId), true);
    const waitingWorkflow = compileWoml(parseWoml(`<woml>
      <workflow id="rcm13-outcome" version="1.0.0">
        <triggers><manual id="start" /></triggers>
        <steps><wait id="pause" duration="1s" />
          <step id="done"><script>return { ok: true };</script></step>
        </steps>
      </workflow></woml>`, { file: 'rcm13-outcome.woml' }));
    const outcome = json(await promise(native.executeWomlWorkflowDurableOutcome!(
      JSON.stringify(waitingWorkflow), compiledDefinitionHash(waitingWorkflow), '{}', executable, host, 5_000,
      join(directory, 'outcome.sqlite'), '{}', '[]', undefined,
    )));
    assert.ok(JSON.stringify(outcome).includes('waiting'));

    const webhook = compileWoml(parseWoml(`<woml>
      <workflow id="rcm13-ingress" version="1.0.0">
        <triggers><webhook id="arrival" path="/rcm13" method="POST" auth="none">
          <schema>{"type":"object","additionalProperties":false}</schema>
        </webhook></triggers>
        <steps><step id="hello"><script>return { ok: true };</script></step></steps>
      </workflow></woml>`, { file: 'rcm13-ingress.woml' }));
    const ingressState = join(directory, 'ingress.sqlite');
    const registrations = JSON.stringify([{
      workflow: webhook, definitionHash: compiledDefinitionHash(webhook),
      resolvedSecrets: {}, runtimeModules: [],
    }]);
    const observations: string[] = [];
    const start = (address: string) => promise(native!.startWomlWebhookRuntime!(
      registrations, '{}', address, ingressState, executable, host, 5_000, 5_000,
      'rcm13-deployment', compiledDefinitionHash(webhook), true, undefined, undefined,
      true, false, 1, (value: string) => { observations.push(value); },
    ));
    const started = json(await start('127.0.0.1:0'));
    runtimeId = started.runtimeId as string;
    assert.equal(typeof runtimeId, 'string');
    const address = `${started.host}:${started.port}`;
    const post = () => fetch(`http://${address}/rcm13`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: '{}',
      signal: AbortSignal.timeout(5_000),
    });
    const closed = await post();
    assert.equal(closed.status, 503);
    await closed.arrayBuffer();
    await promise(native.activateWomlWebhookRuntime!(runtimeId));
    const admitted = await post();
    assert.equal(admitted.status, 202);
    const accepted = await admitted.json() as { runId: string };
    await eventually(() => json(native!.inspectWomlRun!(ingressState, accepted.runId)).status === 'succeeded', 'webhook run');
    await eventually(() => observations.some(value => value.includes('woml.trigger-progress')), 'trigger callback');
    await eventually(() => observations.some(value => value.includes('woml.telemetry-signal/v1')), 'telemetry callback');
    observations.forEach(value => json(value));
    await promise(native.stopWomlWebhookRuntime!(runtimeId));
    const stoppedId = runtimeId;
    runtimeId = undefined;
    await assert.rejects(promise(native.activateWomlWebhookRuntime!(stoppedId)), /does not exist/);
    // Rebinding the exact address/store proves shutdown joined the owner and
    // released both the listener and durable runtime ownership.
    const restarted = json(await start(address));
    runtimeId = restarted.runtimeId as string;
    assert.notEqual(runtimeId, stoppedId);
    await promise(native.stopWomlWebhookRuntime!(runtimeId));
    runtimeId = undefined;
    return { surface, inspectionCode, ordinary: true, durable: true, outcome: true,
      callback: true, activation: true, telemetry: true, shutdownAndRebind: true };
  } finally {
    if (runtimeId !== undefined && native !== undefined) {
      await promise(native.stopWomlWebhookRuntime!(runtimeId));
    }
    try {
      await rm(directory, { recursive: true, force: true, maxRetries: 10, retryDelay: 50 });
    } catch (error: unknown) {
      // Windows cannot unlink a loaded DLL. Only tolerate that exact leftover,
      // not leaked databases/listeners or arbitrary cleanup errors.
      const code = (error as NodeJS.ErrnoException).code;
      if (process.platform !== 'win32' || native === undefined
          || !['EPERM', 'EBUSY', 'EACCES', 'ENOTEMPTY'].includes(code ?? '')) throw error;
      assert.deepEqual(await readdir(directory), ['candidate.node']);
      console.warn('RCM native verification: Windows retains the loaded addon copy until runner cleanup.');
    }
  }
}

if (import.meta.main) {
  const sources = process.argv.slice(2);
  assert.ok(sources.length <= 2,
    'Usage: verify-rust-core-native.ts [addon] [second-addon-for-comparison]');
  if (sources.length === 0) sources.push(defaultStagedNativePath());
  // This also bounds failures in unref'ed callbacks or native lifecycle joins.
  const watchdog = setTimeout(() => {
    console.error('RCM13 real-addon contract watchdog expired.');
    process.exit(1);
  }, 60_000);
  try {
    const first = await verifyRustCoreNative(sources[0]!);
    if (sources[1] !== undefined) assert.deepEqual(await verifyRustCoreNative(sources[1]), first);
    console.log(`RCM13: ${expectedNativeExports.length} real exports; sync/async/error/callback, activation, telemetry, shutdown and rebind verified${sources.length === 2 ? ' identically for both addons' : ''}.`);
  } finally {
    clearTimeout(watchdog);
  }
}
