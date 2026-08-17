#!/usr/bin/env bun

import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const packageRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(packageRoot, '..');
const executable = resolve(packageRoot, 'dist/cli.js');
const budget = (await Bun.file(
  resolve(projectRoot, 'examples/fork-branch-performance-budgets.v1.json')
).json()) as { maximumMedianExecutionMs: number };
const root = await mkdtemp(join(tmpdir(), 'woml-fork-benchmark-'));

try {
  const workflowPath = join(root, 'distribution.woml');
  await writeFile(
    workflowPath,
    await readFile(resolve(projectRoot, 'examples/forkDistributionWorkflow.woml'), 'utf8')
  );
  const samples: number[] = [];
  for (let index = 0; index < 3; index += 1) {
    const started = performance.now();
    const result = Bun.spawnSync(
      [
        process.execPath,
        executable,
        'test',
        workflowPath,
        '--state',
        join(root, `state-${index}.sqlite`),
      ],
      { cwd: root, stdout: 'pipe', stderr: 'pipe' }
    );
    const elapsed = performance.now() - started;
    if (result.exitCode !== 0) throw new Error(result.stderr.toString());
    const output = JSON.parse(result.stdout.toString()) as { published?: unknown[] };
    if (output.published?.length !== 4) {
      throw new Error('The fork benchmark did not publish all four branch results.');
    }
    samples.push(elapsed);
  }
  samples.sort((left, right) => left - right);
  const medianExecutionMs = samples[1]!;
  if (medianExecutionMs > budget.maximumMedianExecutionMs) {
    throw new Error(
      `Fork execution exceeded its budget: ${medianExecutionMs.toFixed(2)} ms > ${budget.maximumMedianExecutionMs} ms.`
    );
  }
  console.log(
    JSON.stringify({
      profile: 'woml.fork-branch-performance-results/v1',
      branches: 4,
      stepsPerBranch: 2,
      samplesMs: samples.map(value => Number(value.toFixed(2))),
      medianExecutionMs: Number(medianExecutionMs.toFixed(2)),
      maximumMedianExecutionMs: budget.maximumMedianExecutionMs,
    })
  );
} finally {
  await rm(root, { recursive: true, force: true });
}
