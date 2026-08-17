#!/usr/bin/env bun

import { stat } from 'node:fs/promises';
import { resolve } from 'node:path';

const workflowPath = resolve(process.argv[2] ?? 'hello.woml');
const cliPath = resolve(import.meta.dir, '../dist/cli.js');

try {
  await stat(cliPath);
} catch {
  process.stderr.write(
    'The WOML CLI is not built. Run "cd woml-cli && bun run build" first.\n',
  );
  process.exit(1);
}

const startedAt = performance.now();
const child = Bun.spawn([cliPath, 'run', workflowPath], {
  stdout: 'pipe',
  stderr: 'pipe',
});

let firstOutputAt: number | undefined;

const stdoutTask = (async () => {
  const reader = child.stdout.getReader();
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    if (firstOutputAt === undefined) firstOutputAt = performance.now();
    process.stdout.write(value);
  }
})();

const stderrTask = (async () => {
  const reader = child.stderr.getReader();
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    process.stderr.write(value);
  }
})();

const exitCode = await child.exited;
await Promise.all([stdoutTask, stderrTask]);
const finishedAt = performance.now();

if (firstOutputAt !== undefined) {
  process.stderr.write(
    `[measurement] JSON appeared after ${(firstOutputAt - startedAt).toFixed(2)} ms\n`,
  );
} else {
  process.stderr.write('[measurement] The workflow produced no JSON output.\n');
}
process.stderr.write(
  `[measurement] Process finished after ${(finishedAt - startedAt).toFixed(2)} ms\n`,
);

process.exitCode = exitCode;
