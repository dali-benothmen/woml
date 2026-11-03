#!/usr/bin/env bun
// SRS3 correctness/structural gate, not a wall-time benchmark or package gate.
import assert from 'node:assert/strict';
import { Database } from 'bun:sqlite';
import { copyFile, mkdtemp, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { compileWoml, parseWoml } from '@woml/compiler';
import {
  activateWebhookRuntimeWithRust, compiledDefinitionHash, executeWorkflowWithRustDurable,
  executeApprovalWorkflowWithRust, resumeApprovalWorkflowWithRust, settleTimerWaitWithRust,
  startWebhookRuntimeWithRust, stopWebhookRuntimeWithRust, submitManualTriggerWithRust,
  inspectRunArtifactsWithRust,
  type RuntimeCapabilitiesV1,
} from '../src/rust-executor';
import { TerminalObserver } from './runtime-session-profile-child';
import { runtimeProfileFixtures, runtimeProfilePolicy } from './runtime-profile-fixtures';
import { cleanupNativeVerificationDirectory } from './verify-rust-core-native';
import { defaultStagedNativePath } from './verify-native-exports';

interface Event { type: string; sequence: number; data: Record<string, unknown> }
function history(path: string, runId: string): Event[] {
  const db = new Database(path, { readonly: true });
  try {
    return (db.query('SELECT event_json FROM woml_run_events WHERE run_id=?1 ORDER BY sequence').all(runId) as { event_json: string }[])
      .map(row => JSON.parse(row.event_json) as Event);
  } finally { db.close(); }
}
// Identity/timing values intentionally differ; compare accepted fact classes,
// exact node/attempt/operation identities, terminal results and index contents.
function facts(events: Event[]): string[] {
  return events.map(event => JSON.stringify({ type: event.type,
    ...Object.fromEntries(['nodeId', 'attempt', 'capability', 'operation', 'hookId', 'sagaId', 'actionId', 'outcome']
      .filter(key => key in event.data).map(key => [key, event.data[key]])),
  })).sort();
}
function terminal(events: Event[]): unknown {
  assert.equal(events.at(-1)?.type, 'run_finalized');
  assert.deepEqual(events.map(event => event.sequence), events.map((_, i) => i + 1));
  return events.find(event => event.type === 'run_outcome_decided')?.data.result;
}
function indexFacts(path: string): unknown {
  const db = new Database(path, { readonly: true });
  try {
    return {
      integrity: db.query('PRAGMA integrity_check').values(),
      foreignKeys: db.query('PRAGMA foreign_key_check').values(),
      openClaims: db.query('SELECT COUNT(*) FROM woml_scheduler_claims').values(),
    };
  } finally { db.close(); }
}

async function main() {
  const directory = process.argv[3]!;
  const selected = process.argv[5]!;
  const nativeCorePath = join(directory, 'candidate.node');
  const trace = join(directory, 'trace.jsonl');
  await copyFile(resolve(process.argv[4]!), nativeCorePath);
  const common = { nativeCorePath, bunExecutable: Bun.which('bun')!,
    scriptHostPath: resolve(import.meta.dir, '../src/script-host.ts'), scriptTimeoutMs: 30_000 };
  let active: string | undefined;
  const fullOpens = async () => (await readFile(trace, 'utf8')).split('\n').filter(Boolean)
    .map(line => JSON.parse(line) as { name: string }).filter(span => span.name === 'sqlite.open_initialize').length;
  try {
    const fixtures = [
      ...runtimeProfileFixtures().filter(f => ['sequential-2', 'storage-4', 'state-8', 'parallel-8', 'foreach-8', 'hooks-8'].includes(f.id)),
      { id: 'retry-wait-output', governed: false, expected: { value: 3 }, source: `<woml><workflow id="runtime-profile-retry-wait-output">
        <config concurrency="1" timeout="3m" />
        <lifecycle><on-start><script>console.log("prepared hook output");</script></on-start><on-complete><script>return null;</script></on-complete></lifecycle>
        <triggers><manual id="start" /></triggers><steps>
        <step id="retry" retry="3" retry-delay="10ms"><script>if (attempt.number < 3) throw new Error("expected transient"); return { value: attempt.number };</script></step>
        <wait id="pause" duration="1s" />
        <step id="done"><script>return context.steps.retry;</script></step>
        </steps></workflow></woml>` },
    ];
    assert.ok(selected === 'ownership' || fixtures.some(fixture => fixture.id === selected), 'unknown native verification case');
    for (const fixture of fixtures.filter(fixture => fixture.id === selected)) {
      const id = fixture.id;
      const workflowId = `runtime-profile-${id.replace(/-(\d+)/gu, '-n$1')}`;
      const source = fixture.source.replace(`id="runtime-profile-${id}"`, `id="${workflowId}"`);
      const workflow = compileWoml(parseWoml(source));
      const runtimeCapabilities: RuntimeCapabilitiesV1 | undefined = fixture.governed ? {
        infrastructurePolicy: { required: true, trust: {}, policy: { ...runtimeProfilePolicy,
          rules: runtimeProfilePolicy.rules.map(rule => ({ ...rule, match: { ...rule.match, workflows: [workflowId] } })),
        } },
      } : undefined;
      const options = { ...common, runtimeCapabilities };
      const referencePath = join(directory, `${id}-reference.sqlite`);
      const executeReference = async () => {
        if (id !== 'retry-wait-output') {
          return executeWorkflowWithRustDurable(workflow, referencePath, { ...options, trigger: {} });
        }
        let outcome = await executeApprovalWorkflowWithRust(workflow, referencePath, { ...options, trigger: {} });
        // The unowned reference has no background wait scheduler. Settle only
        // its exact due timer, then use the public one-shot continuation API.
        if (outcome.status === 'waiting_time') {
          await Bun.sleep(Math.max(0, Date.parse(outcome.wait.deadline) - Date.now()) + 20);
          assert.notEqual(settleTimerWaitWithRust(referencePath, outcome.runId, outcome.wait.waitId, common).status, 'not_due');
          outcome = await resumeApprovalWorkflowWithRust(workflow, referencePath, outcome.runId, options);
        }
        assert.ok(outcome.status === 'succeeded', JSON.stringify(outcome));
        return outcome.execution;
      };
      const reference = await executeReference();
      assert.deepEqual(reference.result, fixture.expected);
      const path = `${directory}/./${id}-prepared.sqlite`;
      const observer = new TerminalObserver();
      const runtime = await startWebhookRuntimeWithRust([{ workflow, definitionHash: compiledDefinitionHash(workflow), resolvedSecrets: {}, runtimeModules: [] }], path,
        { ...options, host: '127.0.0.1', port: 0, startSuspended: true, shutdownTimeoutMs: 10_000,
          deploymentId: 'prepared-runtime-safety', telemetryEnabled: false, onTriggerProgress: progress => observer.record(progress) });
      active = runtime.runtimeId;
      const request = { profile: 'woml.manual-trigger-admission/v1' as const, type: 'request' as const,
        requestId: `request-${id}`, workflowId, triggerId: 'start', payload: {}, requestedAt: new Date().toISOString() };
      const beforeActivation = new Database(path, { readonly: true });
      try { assert.deepEqual(beforeActivation.query('SELECT COUNT(*) FROM woml_runs').values(), [[0]]); } finally { beforeActivation.close(); }
      await activateWebhookRuntimeWithRust(active, common);
      let opens = await fullOpens();
      assert.ok(opens >= 2, 'full initialization must still occur at startup');
      if (id === 'sequential-2') {
        // Observe two real owner heartbeats, without polling native inspection
        // (which deliberately retains the standalone full-opening contract).
        await Bun.sleep(4_500);
        assert.equal(await fullOpens(), opens, 'idle maintenance must not initialize');
      }
      const accepted = await submitManualTriggerWithRust(active, request, common);
      assert.equal(accepted.type, 'accepted', JSON.stringify(accepted));
      assert.ok(accepted.type === 'accepted');
      assert.equal((await observer.wait(accepted.runId, 30_000)).status, 'succeeded');
      const duplicate = await submitManualTriggerWithRust(active, request, common);
      assert.ok(duplicate.type === 'accepted');
      assert.equal(duplicate.runId, accepted.runId);
      const events = history(path, accepted.runId);
      assert.deepEqual(terminal(events), fixture.expected);
      assert.deepEqual(facts(events), facts(history(referencePath, reference.runId)));
      assert.equal(await fullOpens(), opens, `${id}: admitted execution must not initialize`);
      if (id === 'sequential-2') {
        const schema = () => {
          const db = new Database(path, { readonly: true });
          try { return db.query('PRAGMA schema_version').values(); } finally { db.close(); }
        };
        const cookie = schema();
        assert.deepEqual(inspectRunArtifactsWithRust(path, accepted.runId, common).artifacts, []);
        assert.deepEqual(schema(), cookie, 'standalone inspection must not add schema after publication');
        // Standalone operator inspection deliberately keeps its full opening
        // contract. Exclude that explicit call, not any owned request work.
        opens = await fullOpens();
        const later = await submitManualTriggerWithRust(active, { ...request, requestId: 'later-independent-request' }, common);
        assert.ok(later.type === 'accepted');
        assert.notEqual(later.runId, accepted.runId);
        assert.equal((await observer.wait(later.runId, 30_000)).status, 'succeeded');
        assert.deepEqual(terminal(history(path, later.runId)), fixture.expected);
        assert.equal(await fullOpens(), opens, 'a later independent request must not initialize');
      }
      await stopWebhookRuntimeWithRust(active, common);
      active = undefined;
      assert.deepEqual(indexFacts(path), indexFacts(referencePath));
      assert.equal(await fullOpens(), opens, 'owned shutdown must not initialize');
      console.log(`Prepared/reference equivalence: ${id} (${events.length} accepted events)`);
    }

    if (selected !== 'ownership') return;
    // Actual N-API fault: a successor row must block the old admission and must
    // survive the old runtime's heartbeat failure and bounded shutdown.
    const workflow = compileWoml(parseWoml(runtimeProfileFixtures()[0]!.source.replace('runtime-profile-sequential-2', 'prepared-owner-transfer')));
    const state = join(directory, 'owner-transfer.sqlite');
    const registrations = [{ workflow, definitionHash: compiledDefinitionHash(workflow), resolvedSecrets: {}, runtimeModules: [] }];
    const started = await startWebhookRuntimeWithRust(registrations, state, { ...common, host: '127.0.0.1', port: 0,
      startSuspended: true, deploymentId: 'prepared-runtime-safety', shutdownTimeoutMs: 5_000 });
    active = started.runtimeId;
    await activateWebhookRuntimeWithRust(active, common);
    // Bun and the addon embed independent SQLite implementations. Inject from
    // another process so closing a Bun descriptor cannot release this process's
    // native POSIX SQLite locks and race its rollback-journal cleanup.
    const injected = Bun.spawn([process.execPath, '-e', `
      import { Database } from 'bun:sqlite';
      const db = new Database(${JSON.stringify(state)});
      db.exec('PRAGMA busy_timeout=5000; BEGIN IMMEDIATE');
      db.run("UPDATE woml_runtime_owner SET runtime_instance_id='successor-runtime'");
      db.exec('COMMIT'); db.close();
    `], { stdout: 'pipe', stderr: 'pipe' });
    const injectionDeadline = setTimeout(() => injected.kill(), 10_000);
    try {
      const [stdout, stderr] = await Promise.all([new Response(injected.stdout).text(), new Response(injected.stderr).text()]);
      assert.equal(await injected.exited, 0, `${stderr}\n${stdout}`);
    } finally {
      clearTimeout(injectionDeadline);
      if (injected.exitCode === null) { injected.kill(); await injected.exited; }
    }
    let rejected = false;
    try {
      const result = await submitManualTriggerWithRust(active, { profile: 'woml.manual-trigger-admission/v1', type: 'request', requestId: 'after-transfer',
        workflowId: workflow.workflowId, triggerId: 'start', payload: {}, requestedAt: new Date().toISOString() }, common);
      rejected = result.type === 'rejected';
    } catch (error) {
      assert.match(String(error), /PREPARATION_INVALID|unavailable|stopping|closed/i);
      rejected = true;
    }
    assert.equal(rejected, true);
    await stopWebhookRuntimeWithRust(active, common);
    active = undefined;
    const after = new Database(state, { readonly: true });
    try {
      assert.equal((after.query('SELECT COUNT(*) AS count FROM woml_runs').get() as { count: number }).count, 0);
      assert.equal((after.query('SELECT runtime_instance_id AS owner FROM woml_runtime_owner').get() as { owner: string }).owner, 'successor-runtime');
    } finally { after.close(); }
    console.log('Prepared native safety: owner transfer refused, successor preserved, no unadmitted run.');
  } finally {
    if (active) await stopWebhookRuntimeWithRust(active, common);
  }
}

async function supervise() {
  assert.ok(process.argv.length <= 3, 'Pass at most one explicit native artifact');
  for (const selected of ['sequential-2', 'hooks-8', 'state-8', 'storage-4', 'parallel-8', 'foreach-8', 'retry-wait-output', 'ownership']) {
    const directory = await mkdtemp(join(tmpdir(), `woml-prepared-native-${selected}-`));
    const child = Bun.spawn([process.execPath, import.meta.path, '--child', directory,
      resolve(process.argv[2] ?? defaultStagedNativePath()), selected], {
      env: { ...process.env, RUST_MIN_STACK: undefined, WOML_PROFILE: '1', WOML_PROFILE_OUTPUT: join(directory, 'trace.jsonl'), WOML_PROFILE_TRACE_ID: 'prepared-runtime-safety' },
      stdout: 'inherit', stderr: 'inherit', detached: process.platform !== 'win32',
    });
    // Environment is supplied at process creation: changing Bun's JS env after
    // loading an addon is not evidence that Rust's process environment changed.
    const watchdog = setTimeout(() => {
      console.error(`Prepared native verifier timed out; diagnostics: ${directory}`);
      if (process.platform === 'win32') Bun.spawnSync(['taskkill', '/PID', String(child.pid), '/T', '/F']);
      else { try { process.kill(-child.pid, 'SIGKILL'); } catch { child.kill(); } }
    }, 120_000);
    const code = await child.exited;
    clearTimeout(watchdog);
    if (code === 0) await cleanupNativeVerificationDirectory(directory, false);
    else {
      // A crashed coordinator may leave its host descendants behind. On Unix
      // the isolated group remains addressable after its leader exits.
      if (process.platform !== 'win32') {
        try { process.kill(-child.pid, 'SIGKILL'); }
        catch (error) { if ((error as NodeJS.ErrnoException).code !== 'ESRCH') throw error; }
      }
      console.error(`Prepared native verifier failed; retained diagnostics: ${directory}`);
      process.exitCode = code || 1;
      return;
    }
  }
}

if (import.meta.main) {
  if (process.argv[2] === '--child') await main();
  else await supervise();
}
