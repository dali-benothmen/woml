#!/usr/bin/env bun

import { readdir } from 'node:fs/promises';
import { resolve } from 'node:path';

const cliRoot = resolve(import.meta.dir, '..');
const testsRoot = resolve(cliRoot, 'tests');

export const cliTestShardNames = [
  'contracts',
  'workflows',
  'providers',
  'operations',
] as const;
type ShardName = (typeof cliTestShardNames)[number];

function shardFor(file: string): ShardName {
  if (/(?:slack|telegram|discord|whatsapp|notification|provider)/u.test(file)) {
    return 'providers';
  }
  if (
    /(?:production|runtime|observability|backup|retention|log-follow|background|foreground|run-management|manual-trigger|presentation)/u.test(file)
  ) {
    return 'operations';
  }
  if (
    /(?:contract|authoring|foundation|protocol|identity|secret-prompt|native-platform|architecture)/u.test(file)
  ) {
    return 'contracts';
  }
  return 'workflows';
}

const requestedIndex = process.argv.indexOf('--shard');
const requested = requestedIndex < 0 ? undefined : process.argv[requestedIndex + 1];
if (
  requested !== undefined &&
  !cliTestShardNames.includes(requested as ShardName)
) {
  throw new Error(`Unknown CLI shard ${requested}. Expected ${cliTestShardNames.join(', ')}.`);
}

const files = (await readdir(testsRoot))
  .filter(file => file.endsWith('.test.ts'))
  .sort();
const shards = new Map<ShardName, string[]>(
  cliTestShardNames.map(name => [name, []]),
);
for (const file of files) shards.get(shardFor(file))!.push(`tests/${file}`);

const selected = requested === undefined
  ? cliTestShardNames
  : [requested as ShardName];
for (const name of selected) {
  const shardFiles = shards.get(name)!;
  const started = performance.now();
  const result = Bun.spawnSync({
    cmd: [
      process.execPath,
      'test',
      ...shardFiles,
      '--max-concurrency=1',
      '--timeout=30000',
    ],
    cwd: cliRoot,
    stdout: 'pipe',
    stderr: 'pipe',
  });
  if (result.exitCode !== 0) {
    process.stdout.write(result.stdout.toString());
    process.stderr.write(result.stderr.toString());
    throw new Error(`CLI ${name} shard failed.`);
  }
  process.stdout.write(
    `[cli:${name}] ${shardFiles.length} files passed in ${Math.round(performance.now() - started)} ms\n`,
  );
}

process.stdout.write(`[cli] ${files.length} test files are assigned to bounded shards\n`);
