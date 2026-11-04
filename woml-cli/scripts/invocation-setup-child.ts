// One isolated harness process. Real built host/workers; no SQLite or native addon.
import assert from 'node:assert/strict';
import { join } from 'node:path';
import contract from '../../docs/invocation-setup-acceptance.v1.json';
import { encodeFrame, FrameDecoder } from '../src/script-host/framing';
import { isScriptHostMessage } from '../src/script-host/protocol';
import { cohorts, fixture, type Cohort } from './invocation-setup-fixtures';
import { resourceRecord } from './validated-state-baseline';

export async function measureCohort(hostPath: string, cohort: Cohort, directory: string, trace = false, smoke = false) {
  const spec = fixture(cohort), decoder = new FrameDecoder({ maxFrameBytes: 8 * 1024 * 1024 });
  const tracePath = join(directory, `${cohort}.ndjson`);
  const started = performance.now();
  const child = Bun.spawn([process.execPath, hostPath], {
    env: { WOML_SCRIPT_HOST_PROTOCOL_VERSION: '12',
      ...(trace ? { WOML_PROFILE: '1', WOML_PROFILE_PROCESS: 'host', WOML_PROFILE_TRACE_ID: `isr-${cohort}`, WOML_PROFILE_OUTPUT: tracePath } : {}) },
    stdin: 'pipe', stdout: 'pipe', stderr: 'pipe',
  });
  const stderr = new Response(child.stderr).text();
  let fail: (reason: unknown) => void = () => {};
  const failed = new Promise<never>((_, reject) => { fail = reject; });
  // Attach immediately so an early pipe failure never becomes an unhandled rejection.
  void failed.catch(() => {});
  const pending = new Map<string, { resolve: (value: any) => void; reject: (error: unknown) => void }>();
  const completed = new Set<string>();
  let readyResolve: (value: any) => void = () => {};
  const ready = new Promise<any>(resolve => { readyResolve = resolve; });
  let readySeen = false, ending = false, readError: unknown;
  const watchdog = setTimeout(() => { fail(new Error(`ISR ${cohort} watchdog`)); child.kill(); }, contract.cohortWatchdogMs);
  const reading = (async () => {
    try {
      for await (const chunk of child.stdout) for (const raw of decoder.push(chunk)) {
        assert.ok(isScriptHostMessage(raw), 'Invalid host response');
        const message = raw as any;
        assert.equal(message.protocolVersion, 12);
        if (message.messageType === 'ready') {
          assert.equal(readySeen, false); readySeen = true; readyResolve(message); continue;
        }
        const key = message.messageType === 'module_registered' ? message.bundleDigest : message.invocationId;
        assert.ok(pending.has(key), `Unexpected/duplicate response: ${message.messageType}`);
        const receiver = pending.get(key)!; pending.delete(key);
        if (message.messageType === 'completed') {
          assert.ok(!completed.has(key)); completed.add(key);
          assert.deepEqual(message.outcome, { kind: 'success', value: 1 }, 'Fresh worker/module must return exactly one');
        } else { assert.equal(message.messageType, 'module_registered'); assert.equal(message.accepted, true); }
        receiver.resolve(message);
      }
      decoder.finish();
      assert.ok(ending && pending.size === 0, 'Host exited before complete drain');
    } catch (error) { readError = error; fail(error); }
  })();
  const exited = child.exited.then(code => {
    if (!ending) fail(new Error(`ISR host exited early: ${code}`));
    return code;
  });
  async function send(message: any) {
    const key = message.messageType === 'register_module' ? message.bundleDigest : message.invocationId;
    assert.ok(!pending.has(key) && !completed.has(key));
    const response = new Promise<any>((resolve, reject) => { pending.set(key, { resolve, reject }); });
    try { child.stdin.write(encodeFrame(message)); await child.stdin.flush(); return await Promise.race([response, failed]); }
    catch (error) { pending.delete(key); throw error; }
  }
  const requestRows: { batch: string; index: number; wallMs: number; hostDurationMs: number }[] = [];
  async function invoke(index: number, batch: string) {
    const request = spec.request(index, batch), before = performance.now();
    const message = await send(request);
    const wallMs = performance.now() - before;
    assert.ok(Number.isFinite(message.durationMs) && message.durationMs >= 0);
    requestRows.push({ batch, index, wallMs, hostDurationMs: message.durationMs });
  }
  try {
    await Promise.race([ready, failed]); const readyMs = performance.now() - started;
    const registrationStarted = performance.now();
    for (const artifact of spec.artifacts) await send(artifact);
    const registrationMs = performance.now() - registrationStarted;
    const firstStarted = performance.now(); await invoke(0, 'first'); const firstInvocationMs = performance.now() - firstStarted;
    const batches = [];
    for (let sample = 0; sample < (smoke ? 1 : trace ? 1 : contract.warmupBatches + contract.measuredBatches); sample++) {
      const before = performance.now();
      const count = smoke ? Math.min(4, spec.count) : trace ? contract.controlInvocationsPerBatch : spec.count;
      for (let index = 0; index < count; index += spec.concurrency) {
        await Promise.all(Array.from({ length: Math.min(spec.concurrency, count - index) }, (_, offset) => invoke(index + offset, `sample${sample}`)));
      }
      batches.push({ sample, warmup: !trace && !smoke && sample < contract.warmupBatches, invocations: count, wallMs: performance.now() - before });
    }
    const teardownStarted = performance.now(); ending = true; await child.stdin.end();
    const exitCode = await Promise.race([exited, failed]); await reading;
    assert.ifError(readError);
    assert.equal(exitCode, 0); const teardownMs = performance.now() - teardownStarted;
    const errors = await stderr; assert.equal(errors, '', 'No script/protocol output may escape to stderr');
    assert.equal(pending.size, 0);
    assert.equal(completed.size, 1 + batches.reduce((sum, row) => sum + row.invocations, 0));
    return { cohort, fixtureSha256: spec.fixtureSha256, concurrency: spec.concurrency, readyMs, registrationMs, firstInvocationMs,
      batches, requestRows, teardownMs, lifetimeMs: performance.now() - started, completed: completed.size,
      hostProcessUsage: resourceRecord(child.resourceUsage()), allocationBytes: null, scopedPeakLiveBytes: null, cpuMs: null,
      usageScope: 'Host process including Bun worker threads as reported by Bun; raw RSS unit unverified, not a scoped heap or process-tree measurement' };
  } finally {
    clearTimeout(watchdog);
    if (!ending || child.exitCode === null) child.kill();
    await child.exited;
    // Consume every stream/task before returning, including errors/watchdogs.
    await reading; await stderr;
  }
}

if (import.meta.main) {
  const [host, directory, mode] = process.argv.slice(2);
  assert.ok(host && directory && ['timing', 'trace', 'smoke'].includes(mode!));
  const rows = [];
  for (const cohort of mode === 'smoke' ? ['mixed'] : cohorts) {
    const row = await measureCohort(host, cohort, directory, mode === 'trace', mode === 'smoke');
    rows.push(row);
    // Checkpoints survive a later failed cohort. No failing sample is retried.
    await Bun.write(join(directory, 'capture.json'), JSON.stringify({ profile: 'woml.invocation-setup-capture/v1', mode, rows, releaseAccepted: false }, null, 2));
  }
}
