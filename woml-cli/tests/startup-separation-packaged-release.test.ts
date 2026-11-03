import { expect, test } from 'bun:test';
import { Database } from 'bun:sqlite';
import { mkdir, mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { installLocalReleaseCandidate } from './helpers/release-candidate';

async function port(): Promise<number> {
  const server = Bun.serve({ hostname: '127.0.0.1', port: 0, fetch: () => new Response() });
  const value = server.port!; await server.stop(true); return value;
}
async function until(check: () => boolean | Promise<boolean>, description: string | (() => string)) {
  const deadline = Date.now() + 30_000;
  while (!(await check())) {
    if (Date.now() > deadline) throw new Error(`Timed out: ${typeof description === 'function' ? description() : description}`);
    await Bun.sleep(30);
  }
}

function durableResult(state: string, runId: string): unknown {
  // The public `get --json` surface reports lifecycle/status, not result
  // bodies. Verify authoritative results separately, read-only, without
  // importing the repository's executor or selecting another native addon.
  const db = new Database(state, { readonly: true });
  try {
    const events = (db.query('SELECT event_json FROM woml_run_events WHERE run_id=?1 ORDER BY sequence').all(runId) as { event_json: string }[])
      .map(row => JSON.parse(row.event_json));
    expect(events.at(-1)?.type).toBe('run_finalized');
    expect(events.map(event => event.sequence)).toEqual(events.map((_, index) => index + 1));
    const outcomes = events.filter(event => event.type === 'run_outcome_decided');
    expect(outcomes).toHaveLength(1); expect(outcomes[0]!.data.outcome).toBe('succeeded');
    return outcomes[0]!.data.result;
  } finally { db.close(); }
}

test('clean installed CLI serves repeated requests, restarts, and restores managed state', async () => {
  const root = await mkdtemp(join(tmpdir(), 'woml-startup-package-'));
  const consumer = join(root, 'consumer'); await mkdir(consumer);
  await Bun.write(join(consumer, 'package.json'), JSON.stringify({ name: 'startup-clean-consumer', private: true }));
  let child: ReturnType<typeof Bun.spawn> | undefined;
  let passed = false;
  let output = '';
  const drains: Promise<void>[] = [];
  const collect = async (stream: ReadableStream<Uint8Array>) => {
    for await (const chunk of stream) output += new TextDecoder().decode(chunk);
  };
  const killTree = () => {
    if (!child || child.exitCode !== null) return;
    if (process.platform === 'win32') Bun.spawnSync(['taskkill', '/PID', String(child.pid), '/T', '/F'], { stdout: 'ignore', stderr: 'ignore' });
    else {
      try { process.kill(-child.pid, 'SIGKILL'); }
      catch (error) { if ((error as NodeJS.ErrnoException).code !== 'ESRCH') throw error; }
    }
  };
  // Includes installation/startup/shutdown, independent of the timing harness.
  const watchdog = setTimeout(killTree, 110_000);
  try {
    const candidate = await installLocalReleaseCandidate(consumer, { cache: join(root, 'cache'), temporary: join(root, 'tmp') });
    const cli = join(consumer, 'node_modules/woml-cli/dist/cli.js');
    expect(await Bun.file(cli).exists()).toBe(true);
    expect(await Bun.file(join(consumer, 'node_modules', candidate.nativePackage, `woml-core.${candidate.target}.node`)).exists()).toBe(true);
    const invoke = async (...args: string[]) => {
      const command = Bun.spawn([process.execPath, cli, ...args], { cwd: consumer, stdin: 'ignore', stdout: 'pipe', stderr: 'pipe', env: { ...process.env, NO_COLOR: '1' } });
      const timeout = setTimeout(() => command.kill(), 30_000);
      try {
        const [stdout, stderr] = await Promise.all([new Response(command.stdout).text(), new Response(command.stderr).text()]);
        expect(await command.exited, `${args.join(' ')}\n${stderr}\n${stdout}\nRuntime:\n${output}`).toBe(0);
        return stdout;
      } finally { clearTimeout(timeout); }
    };
    const workflow = join(consumer, 'repeated.woml');
    await Bun.write(workflow, `<woml><workflow id="startup-package" version="1.0.0">
      <config concurrency="1" timeout="30s" />
      <lifecycle><on-start><script>console.log("package-start-hook");</script></on-start>
      <on-complete><script>console.log("package-complete-hook");</script></on-complete></lifecycle>
      <triggers><webhook id="arrive" path="/repeat" method="POST" auth="none"><schema>{"type":"object","additionalProperties":false}</schema></webhook></triggers>
      <steps><step id="count"><script>
        const count = await services.state.increment("counter", 1, { name: "count" });
        console.log("package-request-" + count.value); return { value: count.value };
      </script></step></steps></workflow></woml>`);
    const state = join(consumer, 'state/state.sqlite');
    const publicPort = await port(), adminPort = await port();
    const config = join(consumer, 'runtime-config.json');
    const configure = async (statePath: string) => Bun.write(config, JSON.stringify({ schemaVersion: 1,
      deploymentName: 'startup-package', statePath, public: { host: '127.0.0.1', port: publicPort },
      admin: { host: '127.0.0.1', port: adminPort }, logging: { format: 'text', directory: join(root, 'logs') },
      observability: { health: true, metrics: true }, retention: { enabled: false } }));
    const start = async (statePath: string) => {
      await configure(statePath);
      const launched = Bun.spawn([process.execPath, cli, 'run', workflow, '--config', config], {
        cwd: consumer, stdin: 'ignore', stdout: 'pipe', stderr: 'pipe', detached: process.platform !== 'win32', env: { ...process.env, NO_COLOR: '1' },
      });
      child = launched;
      drains.push(collect(launched.stdout), collect(launched.stderr));
      await until(async () => {
        if (launched.exitCode !== null) throw new Error(`Installed runtime exited before readiness:\n${output}`);
        try {
          const descriptor = JSON.parse(await readFile(join(statePath, '../runtime.json'), 'utf8')) as { adminUrl: string };
          return (await fetch(`${descriptor.adminUrl}/readyz`)).status === 200;
        } catch { return false; }
      }, 'installed runtime readiness');
    };
    const stop = async (statePath: string) => {
      await invoke('stop', '--state', statePath);
      await until(() => child!.exitCode !== null, 'installed runtime shutdown');
      expect(await child!.exited, output).toBe(0); child = undefined;
    };
    const request = async (statePath: string, key: string, expected: number) => {
      const response = await fetch(`http://127.0.0.1:${publicPort}/repeat`, { method: 'POST',
        headers: { 'content-type': 'application/json', 'idempotency-key': key }, body: '{}' });
      expect(response.status, await response.clone().text()).toBe(202);
      const { runId } = await response.json() as { runId: string }; expect(runId).toMatch(/^run_/u);
      let inspected: { status: string; lifecycleStatus: string; warnings: unknown[] } | undefined;
      await until(async () => {
        inspected = JSON.parse(await invoke('get', runId, '--state', statePath, '--json'));
        if (['failed', 'cancelled'].includes(inspected!.status)) throw new Error(JSON.stringify(inspected));
        return inspected!.status === 'succeeded' && inspected!.lifecycleStatus === 'completed';
      }, () => `installed run settlement (${runId}); inspection=${JSON.stringify(inspected)}; runtime output: ${output}`);
      expect(inspected!.warnings).toEqual([]);
      expect(durableResult(statePath, runId)).toEqual({ value: expected });
      const expectedOutput = ['package-start-hook', 'package-complete-hook', `package-request-${expected}`];
      let logs = '';
      await until(async () => {
        logs = await invoke('logs', runId, '--state', statePath, '--no-follow', '--json');
        return expectedOutput.every(text => logs.includes(text));
      }, 'persisted hook and request output');
      for (const text of expectedOutput) expect(logs).toContain(text);
      return runId;
    };
    await start(state);
    const first = await request(state, 'first', 1);
    await request(state, 'second', 2);
    // Same delivery is idempotent even after the run has finalized.
    const duplicate = await fetch(`http://127.0.0.1:${publicPort}/repeat`, { method: 'POST', headers: { 'content-type': 'application/json', 'idempotency-key': 'first' }, body: '{}' });
    expect((await duplicate.json() as { runId: string }).runId).toBe(first);
    await stop(state); await start(state); await request(state, 'after-restart', 3); await stop(state);
    const backup = join(root, 'backup');
    expect(JSON.parse(await invoke('backup', backup, '--state', state, '--json')).verified).toBe(true);
    const restored = join(consumer, 'restored/state.sqlite');
    await invoke('restore', backup, '--state', restored, '--json');
    expect(JSON.parse(await invoke('get', first, '--state', restored, '--json')).status).toBe('succeeded');
    expect(durableResult(restored, first)).toEqual({ value: 1 });
    await start(restored); await request(restored, 'after-restore', 4); await stop(restored);
    const probe = Bun.serve({ hostname: '127.0.0.1', port: publicPort, fetch: () => new Response() });
    await probe.stop(true); // The installed owner actually released its listener.
    passed = true;
  } catch (error) {
    console.error(`Installed startup diagnostics retained: ${root}\n${output}`);
    throw error;
  } finally {
    clearTimeout(watchdog); killTree();
    if (child) await child.exited;
    await Promise.all(drains);
    if (passed) await rm(root, { recursive: true, force: true, maxRetries: process.platform === 'win32' ? 10 : 0, retryDelay: 50 });
  }
}, 120_000);
