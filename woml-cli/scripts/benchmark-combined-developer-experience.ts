#!/usr/bin/env bun

import { createHash, randomUUID } from 'node:crypto';
import { mkdir, mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { compileWoml, parseWoml } from '@woml/compiler';

import { publicJavaScriptFiles } from './release-contract';
import { runCli } from '../src/cli';
import { localNativeBinaryName } from '../src/native-platform';
import { resolveWomlProject } from '../src/project';
import {
  executeWorkflowWithRustDurable,
  prepareRemoteAttemptWithRust,
} from '../src/rust-executor';

const root = resolve(import.meta.dir, '../..');
const cliRoot = resolve(root, 'woml-cli');
const budgets = await Bun.file(resolve(
  root,
  'woml/tests/fixtures/combined-developer-experience/budgets.v1.json'
)).json();
const temporary = await mkdtemp(join(tmpdir(), 'woml-cdx-benchmark-'));

function percentile(values: readonly number[], fraction: number): number {
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.max(0, Math.ceil(sorted.length * fraction) - 1)]!;
}

function requireMaximum(label: string, value: number, maximum: number): void {
  if (!Number.isFinite(value) || value > maximum) {
    throw new Error(`${label} exceeded: ${value.toFixed(2)} > ${maximum}.`);
  }
}

function digest(value: string): string {
  return `sha256:${createHash('sha256').update(value).digest('hex')}`;
}

function canonicalJson(value: unknown): string {
  if (value === null || typeof value === 'string' || typeof value === 'boolean')
    return JSON.stringify(value);
  if (typeof value === 'number') return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(',')}]`;
  const record = value as Record<string, unknown>;
  return `{${Object.keys(record).sort().map(key =>
    `${JSON.stringify(key)}:${canonicalJson(record[key])}`
  ).join(',')}}`;
}

async function duration(operation: () => Promise<unknown>): Promise<number> {
  const started = performance.now();
  await operation();
  return performance.now() - started;
}

const workflowSource = (id: string, output = false) => `<woml>
  <workflow id="${id}">
    <triggers><manual id="start" /></triggers>
    <steps><step id="done"><script>
      ${output ? 'for (let index = 0; index < 20; index += 1) console.log("progress-" + index);' : ''}
      return { ok: true };
    </script></step></steps>
  </workflow>
</woml>`;

try {
  const project = join(temporary, 'project');
  const workflows = join(project, 'workflows');
  await mkdir(workflows, { recursive: true });
  for (let index = 0; index < budgets.fixture.workflowFiles; index += 1) {
    await writeFile(
      join(workflows, `workflow-${String(index).padStart(3, '0')}.woml`),
      workflowSource(`benchmark-${index}`)
    );
  }
  const manifest = join(project, 'woml.project.json');
  await writeFile(manifest, JSON.stringify({
    profile: 'woml.project/v1',
    version: 1,
    name: 'cdx-benchmark',
    workflows: ['./workflows'],
  }));

  await resolveWomlProject(manifest);
  if (typeof Bun.gc === 'function') Bun.gc(true);
  const heapBefore = process.memoryUsage().heapUsed;
  const resolutionSamples: number[] = [];
  for (let index = 0; index < 10; index += 1) {
    resolutionSamples.push(await duration(() => resolveWomlProject(manifest)));
  }
  if (typeof Bun.gc === 'function') Bun.gc(true);
  const resolutionHeap = Math.max(0, process.memoryUsage().heapUsed - heapBefore);
  const resolutionP95 = percentile(resolutionSamples, 0.95);
  requireMaximum(
    'Project resolution p95',
    resolutionP95,
    budgets.projectResolution.p95MillisecondsMax
  );
  requireMaximum(
    'Project resolution heap increase',
    resolutionHeap,
    budgets.projectResolution.heapIncreaseBytesMax
  );
  const preflightSamples: number[] = [];
  const silentIo = { stdout: (_value: string) => {}, stderr: (_value: string) => {} };
  for (let index = 0; index < 5; index += 1) {
    const started = performance.now();
    const exitCode = await runCli(
      ['check', '--project', manifest, '--json'],
      silentIo
    );
    if (exitCode !== 0) throw new Error('The benchmark project failed preflight.');
    preflightSamples.push(performance.now() - started);
  }
  const preflightP95 = percentile(preflightSamples, 0.95);
  requireMaximum(
    'Project preflight p95',
    preflightP95,
    budgets.projectPreflight.p95MillisecondsMax
  );

  const nativeCorePath = resolve(
    cliRoot,
    'dist',
    localNativeBinaryName(process.platform, process.arch)
  );
  const scriptHostPath = resolve(cliRoot, 'dist/script-host.js');
  const quiet = compileWoml(parseWoml(workflowSource('activation-quiet')));
  const noisy = compileWoml(parseWoml(workflowSource('activation-noisy', true)));
  const localState = join(temporary, 'local.sqlite');
  const cold = await duration(() => executeWorkflowWithRustDurable(
    quiet,
    localState,
    { nativeCorePath, scriptHostPath }
  ));
  const warmSamples: number[] = [];
  for (let index = 0; index < 5; index += 1) {
    warmSamples.push(await duration(() => executeWorkflowWithRustDurable(
      quiet,
      localState,
      { nativeCorePath, scriptHostPath }
    )));
  }
  const warmP95 = percentile(warmSamples, 0.95);
  requireMaximum(
    'Local cold activation',
    cold,
    budgets.runtime.localColdActivationP95MillisecondsMax
  );
  requireMaximum(
    'Local warm activation p95',
    warmP95,
    budgets.runtime.localWarmActivationP95MillisecondsMax
  );

  const quietSamples: number[] = [];
  const noisySamples: number[] = [];
  for (let index = 0; index < 4; index += 1) {
    quietSamples.push(await duration(() => executeWorkflowWithRustDurable(
      quiet,
      join(temporary, `quiet-${index}.sqlite`),
      { nativeCorePath, scriptHostPath }
    )));
    noisySamples.push(await duration(() => executeWorkflowWithRustDurable(
      noisy,
      join(temporary, `noisy-${index}.sqlite`),
      { nativeCorePath, scriptHostPath }
    )));
  }
  const quietMedian = percentile(quietSamples, 0.5);
  const noisyMedian = percentile(noisySamples, 0.5);
  const streamedOutputOverhead = Math.max(
    0,
    ((noisyMedian - quietMedian) / quietMedian) * 100
  );
  requireMaximum(
    'Streamed-output median overhead percent',
    streamedOutputOverhead,
    budgets.runtime.streamedOutputOverheadPercentMax
  );

  const reviewed = JSON.parse(await readFile(resolve(
    root,
    'woml/tests/fixtures/remote-execution-targets/model.v21.reviewed.json'
  ), 'utf8')) as { graph: { nodes: Record<string, unknown>[] } };
  const node = reviewed.graph.nodes.find(candidate => candidate.id === 'applyRelease');
  if (node === undefined) throw new Error('The reviewed remote node is missing.');
  const remoteSamples: number[] = [];
  for (let index = 0; index < 10; index += 1) {
    const runId = `run_${randomUUID().replaceAll('-', '')}`;
    const invocationId = `inv_${randomUUID().replaceAll('-', '')}`;
    const targetRegistryDigest = digest('cdx-target-registry');
    const identity = {
      domain: 'woml.remote-dispatch/v1', attempt: 1, invocationId,
      iterationIdentity: '', nodeId: 'applyRelease', runId,
      target: 'production-private', targetDigest: targetRegistryDigest,
    };
    const dispatch: Record<string, unknown> = {
      profile: 'woml.dispatch-package/v1', version: 1,
      dispatchId: `dispatch_${digest(canonicalJson(identity)).slice(7)}`,
      runId, nodeId: 'applyRelease', attempt: 1, invocationId,
      target: 'production-private', targetRegistryDigest,
      infrastructurePolicyDigest: digest('cdx-coordinator-policy'),
      definitionPackageRootHash: digest('cdx-definition'), node,
      context: { trigger: {}, steps: { prepare: { release: 'payments-api@42' } } },
      requiredCapabilities: ['runtime.script'], requiredSecrets: [], artifactGrants: [],
      deadline: new Date(Date.now() + 60_000).toISOString().replace('.000Z', 'Z'),
      packageDigest: '',
    };
    dispatch.packageDigest = digest(canonicalJson(dispatch));
    const started = performance.now();
    prepareRemoteAttemptWithRust(
      {
        dispatch,
        coordinatorAuthority: {
          policyDigest: dispatch.infrastructurePolicyDigest,
          allowedCapabilities: [{ id: 'runtime.script', version: 1 }],
          allowedSecretNames: [],
        },
      } as never,
      {
        workerId: 'cdx-benchmark-worker',
        sessionId: `session_${randomUUID().replaceAll('-', '')}`,
        capacity: 1,
        journalPath: join(temporary, `worker-${index}.sqlite`),
        authority: {
          policyDigest: digest('cdx-worker-policy'),
          allowedCapabilities: { 'runtime.script': 1 },
          allowedSecretNames: [],
        },
      },
      { nativeCorePath }
    );
    remoteSamples.push(performance.now() - started);
  }
  const remoteP95 = percentile(remoteSamples, 0.95);
  requireMaximum(
    'Remote dispatch preparation p95',
    remoteP95,
    budgets.runtime.remoteDispatchP95MillisecondsMax
  );

  const sizes = await Promise.all(publicJavaScriptFiles.map(async path => ({
    path,
    bytes: (await stat(resolve(cliRoot, path))).size,
  })));
  const cliBytes = sizes.find(item => item.path === 'dist/cli.js')!.bytes;
  const installedJavaScriptBytes = sizes.reduce((sum, item) => sum + item.bytes, 0);
  requireMaximum(
    'Main bundle increase',
    cliBytes - budgets.baseline.cliBytes,
    budgets.packaging.mainBundleIncreaseBytesMax
  );
  requireMaximum(
    'Installed JavaScript increase',
    installedJavaScriptBytes - budgets.baseline.installedJavaScriptBytes,
    budgets.packaging.installedIncreaseBytesMax
  );
  const cliSource = await Bun.file(resolve(cliRoot, 'dist/cli.js')).text();
  for (const forbidden of [
    'One or more OpenTelemetry providers failed to settle',
    'cdx-benchmark-worker',
  ]) {
    if (cliSource.includes(forbidden)) {
      throw new Error(`An optional implementation was eagerly bundled: ${forbidden}`);
    }
  }
  const optionalIdleRss = 0;
  requireMaximum(
    'Unused optional capability RSS',
    optionalIdleRss,
    budgets.runtime.idleRssIncreaseBytesMax
  );

  console.log(JSON.stringify({
    profile: 'woml.combined-developer-experience-benchmark/v1',
    projectResolutionP95Milliseconds: resolutionP95,
    projectResolutionHeapIncreaseBytes: resolutionHeap,
    projectPreflightP95Milliseconds: preflightP95,
    localColdActivationMilliseconds: cold,
    localWarmActivationP95Milliseconds: warmP95,
    remoteDispatchPreparationP95Milliseconds: remoteP95,
    streamedOutputOverheadPercent: streamedOutputOverhead,
    unusedOptionalCapabilityRssBytes: optionalIdleRss,
    cliBytes,
    installedJavaScriptBytes,
  }, null, 2));
} finally {
  await rm(temporary, { recursive: true, force: true });
}
