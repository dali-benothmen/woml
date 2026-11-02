import { expect, test } from 'bun:test';
import { existsSync } from 'node:fs';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { runCli } from '../src/cli';
import { readRuntimeDescriptor, requestRuntimeStop, runtimeDescriptorPath } from '../src/runtime-control';
import { decodeInspectorAnalytics } from '../src/runtime-inspector/analytics';

const root = resolve(import.meta.dir, '..');
const entry = join(root, 'dist/cli.js');
const native = join(root, 'dist', `woml-core.${process.platform}-${process.arch}.node`);
const nativeTest = existsSync(entry) && existsSync(native) ? test : test.skip;

async function freePort(): Promise<number> {
  const server = Bun.serve({ hostname: '127.0.0.1', port: 0, fetch: () => new Response() });
  const port = server.port!;
  await server.stop(true);
  return port;
}

nativeTest('the built CLI serves real workflow analytics and inspects a file without activating it', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'woml-inspector-native-'));
  const source = join(directory, 'customer-onboarding.woml');
  const state = join(directory, 'state.sqlite');
  const config = join(directory, 'woml.runtime.json');
  const ingress = await freePort();
  await writeFile(source, `<woml>
    <workflow id="customer-onboarding" name="Customer onboarding" description="Create an account and provision its workspace.">
      <triggers><webhook id="newCustomer" path="/inspector-test" method="POST" auth="none"><schema>{"type":"object","additionalProperties":false}</schema></webhook></triggers>
      <steps>
        <step id="createAccount" name="Create account"><script>return {created: true};</script></step>
        <step id="provisionWorkspace" name="Provision workspace"><script>return {provisioned: context.steps.createAccount.created};</script></step>
      </steps>
    </workflow>
  </woml>`);
  await writeFile(config, JSON.stringify({ schemaVersion: 1, statePath: state, admin: { host: '127.0.0.1', port: await freePort() } }));
  const child = Bun.spawn([process.execPath, entry, 'run', source, '--config', config, '--port', String(ingress)], {
    cwd: directory, stdout: 'pipe', stderr: 'pipe',
  });
  // Drain both pipes while the runtime is alive, not only after process exit.
  const stdout = new Response(child.stdout).text();
  const stderr = new Response(child.stderr).text();
  try {
    const path = runtimeDescriptorPath(state);
    const deadline = Date.now() + 15_000;
    while (!(await Bun.file(path).exists()) && Date.now() < deadline && child.exitCode === null) await Bun.sleep(25);
    expect(await Bun.file(path).exists()).toBe(true);
    const descriptor = await readRuntimeDescriptor(path);
    const response = await fetch(`http://127.0.0.1:${ingress}/inspector-test`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: '{}',
    });
    expect(response.ok).toBe(true);
    await response.text();
    const headers = { authorization: `Bearer ${descriptor.capability}` };
    let snapshot: { runs: { status: string }[] } = { runs: [] };
    while (Date.now() < deadline) {
      snapshot = await fetch(`${descriptor.adminUrl}/v1/snapshot`, { headers }).then(value => value.json()) as typeof snapshot;
      if (snapshot.runs.some(run => run.status === 'succeeded')) break;
      await Bun.sleep(25);
    }
    expect(snapshot.runs).toHaveLength(1);
    expect(snapshot.runs[0]!.status).toBe('succeeded');
    const analyticsResponse = await fetch(`${descriptor.adminUrl}/v1/inspector/analytics?workflow=customer-onboarding`, { headers });
    expect(analyticsResponse.status).toBe(200);
    const analytics = decodeInspectorAnalytics(await analyticsResponse.json(), 'customer-onboarding');
    expect(analytics.workflow).toMatchObject({ name: 'Customer onboarding', file: 'customer-onboarding.woml', triggerTypes: ['webhook'] });
    expect(analytics.totals).toMatchObject({ succeeded: 1, failed: 0, retriedRuns: 0 });
    expect(analytics.coverage.detailedRuns).toBe(1);
    expect(analytics.steps.map(step => step.nodeId)).toEqual(['createAccount', 'provisionWorkspace']);
    expect(analytics.steps.every(step => typeof step.averageDurationMs === 'number')).toBe(true);

    const writes: string[] = [], raw: boolean[] = [];
    let input: ((text: string) => void) | undefined;
    expect(await runCli(['inspect', source, '--state', state, '--no-color'], {
      stdout: text => writes.push(text), stderr: text => writes.push(text),
    }, {
      createSecretStore: () => { throw new Error('Inspection must not load secrets.'); },
      readSecret: async () => { throw new Error('Inspection must not resolve secrets.'); },
      inspectorTerminal: {
        isTTY: true, columns: 140, rows: 34,
        write(text) { writes.push(text); if (text.includes('Selected:')) queueMicrotask(() => input?.('q')); },
        setRawMode(enabled) { raw.push(enabled); },
        onInput(listener) { input = listener; return () => { input = undefined; }; },
        onResize() { return () => {}; },
      },
    })).toBe(0);
    expect(raw).toEqual([true, false]);
    expect(writes.join('')).toContain('customer-onboarding');
    const after = await fetch(`${descriptor.adminUrl}/v1/snapshot`, { headers }).then(value => value.json()) as typeof snapshot;
    expect(after.runs).toHaveLength(1);
    await requestRuntimeStop(descriptor);
    expect(await child.exited).toBe(0);
  } finally {
    if (child.exitCode === null) child.kill('SIGKILL');
    await child.exited;
    const diagnostics = (await stdout) + (await stderr);
    if (child.exitCode !== 0) console.error(diagnostics.slice(-4000));
    await rm(directory, { recursive: true, force: true });
  }
}, 30_000);
