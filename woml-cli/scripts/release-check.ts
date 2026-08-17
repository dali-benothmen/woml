#!/usr/bin/env bun

import { resolve } from 'node:path';

const repositoryRoot = resolve(import.meta.dir, '../..');
const cliRoot = resolve(repositoryRoot, 'woml-cli');
const coreRoot = resolve(repositoryRoot, 'core');

interface Command {
  readonly command: readonly string[];
  readonly cwd: string;
}

interface Stage {
  readonly name: string;
  readonly commands: readonly Command[];
}

const bun = process.execPath;
const cargoProfile = [
  '--config',
  'profile.dev.debug=0',
  '--config',
  'profile.dev.incremental=false',
] as const;

const stages: readonly Stage[] = [
  {
    name: 'locked dependencies',
    commands: [{ command: [bun, 'install', '--frozen-lockfile'], cwd: cliRoot }],
  },
  {
    name: 'release build',
    commands: [{ command: [bun, 'run', 'build'], cwd: cliRoot }],
  },
  {
    name: 'format and types',
    commands: [
      { command: ['cargo', 'fmt', '--all', '--', '--check'], cwd: coreRoot },
      { command: [bun, 'run', 'typecheck'], cwd: cliRoot },
    ],
  },
  {
    name: 'language frontend',
    commands: [{ command: [bun, 'test', '../woml/tests', '--max-concurrency=1'], cwd: cliRoot }],
  },
  {
    name: 'portable CLI contracts',
    commands: [
      {
        command: [bun, 'scripts/run-cli-test-shards.ts', '--shard', 'contracts'],
        cwd: cliRoot,
      },
    ],
  },
  {
    name: 'Rust baseline',
    commands: [
      {
        command: [
          'cargo', 'test', ...cargoProfile, '-j', '1', '--locked',
          '-p', 'woml-engine', '--lib', '--', '--test-threads=1',
        ],
        cwd: coreRoot,
      },
      {
        command: [
          'cargo', 'test', ...cargoProfile, '-j', '1', '--locked',
          '-p', 'woml-engine', '--test', 'dag_engine', '--test',
          'durable_event_store', '--test', 'script_host_protocol', '--test',
          'trigger_occurrence', '--test', 'retry_runtime', '--test',
          'runtime_policy_store', '--test',
          'backup_restore', '--', '--test-threads=1',
        ],
        cwd: coreRoot,
      },
      {
        command: [
          'cargo', 'test', '-j', '1', '--locked', '--release',
          '--manifest-path', 'woml-native/Cargo.toml', '--test', 'separation',
          '--', '--test-threads=1',
        ],
        cwd: coreRoot,
      },
    ],
  },
  {
    name: 'architecture and documentation',
    commands: [
      { command: [bun, 'test', 'tests/architecture-separation.test.ts'], cwd: cliRoot },
      { command: [bun, 'scripts/verify-architecture-separation.ts'], cwd: cliRoot },
      { command: [bun, 'run', 'verify:documentation'], cwd: cliRoot },
    ],
  },
  {
    name: 'release automation',
    commands: [
      {
        command: [bun, 'run', 'test:release-automation'],
        cwd: cliRoot,
      },
    ],
  },
  {
    name: 'reproducible package',
    commands: [
      { command: [bun, 'test', 'tests/release-package.test.ts'], cwd: cliRoot },
      { command: [bun, 'run', 'test:package'], cwd: cliRoot },
    ],
  },
];

const completed: Array<{ readonly name: string; readonly duration: number }> = [];
for (const [index, stage] of stages.entries()) {
  const started = performance.now();
  process.stdout.write(`[${index + 1}/${stages.length}] ${stage.name} ... `);
  for (const command of stage.commands) {
    const result = Bun.spawnSync({
      cmd: [...command.command],
      cwd: command.cwd,
      stdout: 'pipe',
      stderr: 'pipe',
      env: { ...process.env, NO_COLOR: '1', CARGO_BUILD_JOBS: '1' },
    });
    if (result.exitCode !== 0) {
      process.stdout.write('failed\n');
      process.stdout.write(result.stdout.toString());
      process.stderr.write(result.stderr.toString());
      process.stderr.write(`\nRelease check stopped in: ${stage.name}\n`);
      process.exit(1);
    }
  }
  const duration = Math.round(performance.now() - started);
  completed.push({ name: stage.name, duration });
  process.stdout.write(`passed (${duration} ms)\n`);
}

const total = completed.reduce((sum, stage) => sum + stage.duration, 0);
process.stdout.write(`\nWOML release check passed ${completed.length} stages in ${total} ms.\n`);
