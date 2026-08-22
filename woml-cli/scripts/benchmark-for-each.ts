#!/usr/bin/env bun

import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const packageRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(packageRoot, '..');
const executable = resolve(packageRoot, 'dist/cli.js');
const budgets = await Bun.file(
  resolve(projectRoot, 'examples/for-each-performance-budgets.v1.json'),
).json() as Record<string, number | string>;
const root = await mkdtemp(join(tmpdir(), 'woml-for-each-benchmark-'));

function source(id: string, itemCount: number, concurrency: number): string {
  const items = JSON.stringify(Array.from({ length: itemCount }, (_, index) => index));
  return `<woml>
  <workflow id="${id}" name="For-each benchmark" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps>
      <step id="load"><script>return { items: ${items} };</script></step>
      <for-each id="process" items="{{context.steps.load.items}}" concurrency="${concurrency}">
        <step id="transform"><script>
          await new Promise(resolve => setTimeout(resolve, context.iteration.index % 4));
          return { index: context.iteration.index, value: context.item * 2 };
        </script></step>
        <result value="{{context.steps.transform}}" />
      </for-each>
      <step id="summary"><script>return context.steps.process;</script></step>
    </steps>
  </workflow>
</woml>\n`;
}

async function measure(
  profile: string,
  itemCount: number,
  concurrency: number,
  maximumMs: number,
): Promise<number> {
  const workflowPath = join(root, `${profile}.woml`);
  await writeFile(workflowPath, source(`for-each-${profile}`, itemCount, concurrency));
  const started = performance.now();
  const result = Bun.spawnSync(
    [
      process.execPath,
      executable,
      'test',
      workflowPath,
      '--state',
      join(root, `${profile}.sqlite`),
    ],
    { cwd: root, stdout: 'pipe', stderr: 'pipe' },
  );
  const elapsedMs = performance.now() - started;
  if (result.exitCode !== 0) throw new Error(result.stderr.toString());
  const output = JSON.parse(result.stdout.toString()) as {
    total: number;
    succeeded: number;
    results: Array<{ index: number; value: number }>;
  };
  if (
    output.total !== itemCount ||
    output.succeeded !== itemCount ||
    output.results.length !== itemCount ||
    output.results.some((item, index) => item.index !== index || item.value !== index * 2)
  ) {
    throw new Error(`${profile} did not preserve complete input-ordered aggregation.`);
  }
  if (!Number.isFinite(elapsedMs) || elapsedMs > maximumMs) {
    throw new Error(
      `${profile} exceeded its budget: ${elapsedMs.toFixed(2)} ms > ${maximumMs} ms.`,
    );
  }
  return elapsedMs;
}

try {
  const largeItemCount = Number(budgets.largeItemCount);
  const concurrentItemCount = Number(budgets.concurrentItemCount);
  const results = {
    profile: 'woml.for-each-performance-results/v1',
    small: {
      items: 3,
      concurrency: 1,
      elapsedMs: await measure('small', 3, 1, Number(budgets.smallMaximumMs)),
      maximumMs: Number(budgets.smallMaximumMs),
    },
    large: {
      items: largeItemCount,
      concurrency: 1,
      elapsedMs: await measure(
        'large',
        largeItemCount,
        1,
        Number(budgets.largeMaximumMs),
      ),
      maximumMs: Number(budgets.largeMaximumMs),
    },
    concurrent: {
      items: concurrentItemCount,
      concurrency: 8,
      elapsedMs: await measure(
        'concurrent',
        concurrentItemCount,
        8,
        Number(budgets.concurrentMaximumMs),
      ),
      maximumMs: Number(budgets.concurrentMaximumMs),
    },
  };
  console.log(JSON.stringify(results, null, 2));
} finally {
  await rm(root, { recursive: true, force: true });
}
