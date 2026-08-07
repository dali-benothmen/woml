#!/usr/bin/env bun

import { existsSync } from 'node:fs';
import { readdir } from 'node:fs/promises';
import { basename, join, resolve } from 'node:path';

interface Check {
  readonly label: string;
  readonly cwd: string;
  readonly command: readonly string[];
}

const cliRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(cliRoot, '..');
const frontendRoot = resolve(projectRoot, 'woml');
const coreManifest = resolve(projectRoot, 'core', 'Cargo.toml');
const engineManifest = resolve(projectRoot, 'core', 'woml-engine', 'Cargo.toml');
const bun = Bun.which('bun');
const cargo = Bun.which('cargo');

if (bun === null || cargo === null) {
  throw new Error('N6 verification requires Bun and Cargo on PATH.');
}

const checks: readonly Check[] = [
  {
    label: 'build the packaged CLI and native Rust core',
    cwd: cliRoot,
    command: [bun, 'run', 'build'],
  },
  {
    label: 'run the WOML frontend suite',
    cwd: frontendRoot,
    command: [bun, 'test'],
  },
  {
    label: 'type-check the WOML frontend',
    cwd: frontendRoot,
    command: [bun, 'run', 'typecheck'],
  },
  {
    label: 'run the Rust workflow-engine suite',
    cwd: projectRoot,
    command: [cargo, 'test', '--manifest-path', engineManifest],
  },
  {
    label: 'check the production Rust N-API bridge library',
    cwd: projectRoot,
    command: [cargo, 'check', '--manifest-path', coreManifest, '--lib'],
  },
  {
    label: 'lint every Rust workflow-engine target',
    cwd: projectRoot,
    command: [
      cargo,
      'clippy',
      '--manifest-path',
      engineManifest,
      '--all-targets',
      '--',
      '-D',
      'warnings',
    ],
  },
  {
    label: 'type-check the packaged CLI and provider host',
    cwd: cliRoot,
    command: [bun, 'run', 'typecheck'],
  },
];

async function runCheck(check: Check): Promise<void> {
  process.stdout.write(`[N6] ${check.label}\n`);
  const child = Bun.spawn(check.command, {
    cwd: check.cwd,
    env: process.env,
    stdin: 'inherit',
    stdout: 'inherit',
    stderr: 'inherit',
  });
  const exitCode = await child.exited;
  if (exitCode !== 0) {
    throw new Error(`N6 verification failed while trying to ${check.label}.`);
  }
}

for (const check of checks) await runCheck(check);

const cliTestFiles = (await readdir(resolve(cliRoot, 'tests')))
  .filter(name => name.endsWith('.test.ts'))
  .sort();
for (const testFile of cliTestFiles) {
  await runCheck({
    label: `run isolated CLI suite ${testFile}`,
    cwd: cliRoot,
    command: [bun, 'test', `./tests/${testFile}`],
  });
}

async function filesBelow(path: string): Promise<string[]> {
  if (!existsSync(path)) return [];
  const files: string[] = [];
  for (const entry of await readdir(path, { withFileTypes: true })) {
    const child = join(path, entry.name);
    if (entry.isDirectory()) files.push(...(await filesBelow(child)));
    else if (entry.isFile()) files.push(child);
  }
  return files;
}

const activeSecrets = Object.entries(process.env).filter(
  ([name, value]) =>
    name.startsWith('WOML_SECRET_') &&
    name !== 'WOML_SECRETS_PROVIDER' &&
    typeof value === 'string' &&
    Buffer.byteLength(value, 'utf8') >= 8
) as Array<[string, string]>;
const publicArtifacts = [
  ...(await filesBelow(resolve(cliRoot, 'dist'))),
  ...(await filesBelow(resolve(cliRoot, 'slack'))),
  ...[
    resolve(projectRoot, '.woml', 'state.sqlite'),
    resolve(projectRoot, '.woml', 'state.sqlite-wal'),
    resolve(projectRoot, '.woml', 'state.sqlite-shm'),
  ].filter(existsSync),
];

for (const [name, secret] of activeSecrets) {
  const needle = Buffer.from(secret);
  for (const path of publicArtifacts) {
    const bytes = Buffer.from(await Bun.file(path).arrayBuffer());
    if (bytes.includes(needle)) {
      throw new Error(
        `N6 secret scan found ${name} in public/durable artifact ${basename(path)}.`
      );
    }
  }
}

process.stdout.write(
  `[N6] release gate passed; scanned ${publicArtifacts.length} public/durable artifacts` +
    (activeSecrets.length === 0
      ? ' (runtime secret scanning covered by generated-sentinel tests).\n'
      : ` against ${activeSecrets.length} active WOML secrets.\n`)
);
