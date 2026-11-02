#!/usr/bin/env bun
// Local demo traffic only. Never imports or invokes an external integration.
import { resolve } from 'node:path';
import { readRuntimeDescriptor } from '../../woml-cli/src/runtime-control';

const mode = process.argv[2] ?? 'history';
if (!['history', 'live', 'status'].includes(mode)) {
  console.error('Usage: bun seed.ts [history|live|status]');
  process.exit(2);
}

const root = import.meta.dir;
const config = await Bun.file(resolve(root, 'woml.runtime.json')).json();
const base = `http://127.0.0.1:${config.public.port}`;
const descriptorPath = resolve(root, '.woml/runtime.json');

interface Run {
  runId: string;
  workflowId: string;
  status: string;
  durationMs: number;
}

async function snapshot(): Promise<Run[]> {
  // Reread the owner-only descriptor; never print its bearer capability.
  const descriptor = await readRuntimeDescriptor(descriptorPath);
  const response = await fetch(`${descriptor.adminUrl}/v1/snapshot`, {
    headers: { authorization: `Bearer ${descriptor.capability}` },
    signal: AbortSignal.timeout(5000),
  });
  if (!response.ok) throw new Error(`Snapshot returned HTTP ${response.status}.`);
  return ((await response.json()) as { runs: Run[] }).runs;
}

async function trigger(path: string, payload: object = {}): Promise<string> {
  const response = await fetch(`${base}/demo/${path}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(payload),
    signal: AbortSignal.timeout(5000),
  });
  if (response.status !== 202) {
    throw new Error(`/demo/${path} returned HTTP ${response.status}; expected durable admission (202).`);
  }
  const { runId } = (await response.json()) as { runId: string };
  console.log(`${path.padEnd(12)} ${runId}`);
  return runId;
}

async function waitForHistory(ids: readonly string[]): Promise<Run[]> {
  const deadline = Date.now() + 60_000;
  while (Date.now() < deadline) {
    const runs = (await snapshot()).filter(run => ids.includes(run.runId));
    if (runs.length === ids.length && runs.every(run => ['succeeded', 'failed', 'cancelled'].includes(run.status))) {
      return runs;
    }
    await Bun.sleep(500);
  }
  throw new Error('The demo runs did not settle in 60 seconds. Check the runtime terminal.');
}

try {
  await snapshot(); // Fail before sending traffic if this runtime is not ready.
  if (mode === 'history') {
    const ids = [];
    for (const fail of [false, false, true]) ids.push(await trigger('onboarding', { fail }));
    ids.push(await trigger('health'));
    const runs = await waitForHistory(ids);
    console.table(runs);
    const onboarding = runs.filter(run => run.workflowId === 'customer-onboarding');
    if (onboarding.filter(run => run.status === 'succeeded').length !== 2 ||
        onboarding.filter(run => run.status === 'failed').length !== 1 ||
        !runs.some(run => run.workflowId === 'service-health' && run.status === 'succeeded')) {
      throw new Error('The history batch did not produce the expected outcomes.');
    }
    console.log('Expected: two onboarding successes, one intentional onboarding failure, one recovered health check.');
    console.log('Select service-health in the inspector to see retries. Analytics can be cached for 15 seconds.');
  } else if (mode === 'live') {
    await Promise.all([
      trigger('welcome'), trigger('welcome'), trigger('welcome'),
      trigger('health'), trigger('review'),
    ]);
    console.log('\nLive traffic admitted. Watch the inspector now: running, queued, retrying, and waiting.');
    console.log('Welcome runs take roughly 9 seconds each, one at a time; document review waits 2 minutes.');
    console.log('Select an active run and press c, then y, to test cancellation.');
  } else {
    console.table(await snapshot());
  }
} catch (error) {
  console.error(`Demo failed: ${error instanceof Error ? error.message : String(error)}`);
  console.error('Start the demo runtime first. See README.md in this folder.');
  process.exitCode = 1;
}
