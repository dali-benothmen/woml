#!/usr/bin/env bun

import { createHash } from 'node:crypto';
import {
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rm,
  stat,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, join, relative, resolve } from 'node:path';

import { womlNativeTargets, nativePackageName } from '../src/native-platform';
import { prepareMainPackage } from './native-release';
import {
  packedPackageFiles,
  publicJavaScriptFiles,
  publicPackageFiles,
  publicSourceMapFiles,
} from './release-contract';

const repositoryRoot = resolve(import.meta.dir, '../..');
const cliRoot = resolve(repositoryRoot, 'woml-cli');
const expectedFiles = [...packedPackageFiles].sort();

interface MainManifest {
  readonly name?: string;
  readonly version?: string;
  readonly author?: string;
  readonly license?: string;
  readonly repository?: Readonly<Record<string, string>>;
  readonly homepage?: string;
  readonly bugs?: Readonly<Record<string, string>>;
  readonly bin?: Readonly<Record<string, string>>;
  readonly files?: readonly string[];
  readonly dependencies?: Readonly<Record<string, string>>;
  readonly devDependencies?: Readonly<Record<string, string>>;
  readonly optionalDependencies?: Readonly<Record<string, string>>;
  readonly scripts?: Readonly<Record<string, string>>;
}

async function filesUnder(root: string): Promise<string[]> {
  const files: string[] = [];
  const visit = async (directory: string): Promise<void> => {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) await visit(path);
      else if (entry.isFile()) files.push(relative(root, path).replaceAll('\\', '/'));
    }
  };
  await visit(root);
  return files.sort();
}

function expectedNativeDependencies(version: string): Record<string, string> {
  return Object.fromEntries(
    womlNativeTargets.map(target => [nativePackageName(target), version]),
  );
}

function sameValues(left: readonly string[], right: readonly string[]): boolean {
  return JSON.stringify([...left].sort()) === JSON.stringify([...right].sort());
}

async function assertNonempty(path: string): Promise<void> {
  if ((await stat(path)).size === 0) throw new Error(`${path} is empty.`);
}

export async function verifyMainPackage(root: string): Promise<void> {
  const manifest = JSON.parse(
    await readFile(resolve(root, 'package.json'), 'utf8'),
  ) as MainManifest;
  if (
    manifest.name !== 'woml-cli' ||
    manifest.version !== '1.0.5' ||
    manifest.author !== 'Mohamed Ali Ben Othmen' ||
    manifest.license !== 'Apache-2.0' ||
    manifest.bin?.woml !== './dist/cli.js'
  ) {
    throw new Error('The staged package does not have the frozen woml-cli@1.0.5 identity.');
  }
  const publicMetadata = JSON.stringify({
    repository: manifest.repository,
    homepage: manifest.homepage,
    bugs: manifest.bugs,
  });
  for (const expected of [
    'github.com/dali-benothmen/woml.git',
    'github.com/dali-benothmen/woml#readme',
    'github.com/dali-benothmen/woml/issues',
  ]) {
    if (!publicMetadata.includes(expected)) {
      throw new Error(`The staged package is missing reviewed metadata: ${expected}.`);
    }
  }
  if (!sameValues(manifest.files ?? [], publicPackageFiles)) {
    throw new Error('The staged package does not use the explicit public file allowlist.');
  }
  if (
    JSON.stringify(manifest.optionalDependencies) !==
    JSON.stringify(expectedNativeDependencies(manifest.version))
  ) {
    throw new Error('The staged package has an invalid native dependency graph.');
  }
  if (
    manifest.dependencies !== undefined ||
    manifest.devDependencies !== undefined ||
    manifest.scripts !== undefined
  ) {
    throw new Error('The staged package contains development or lifecycle dependencies.');
  }

  const files = await filesUnder(root);
  if (!sameValues(files, expectedFiles)) {
    const unexpected = files.filter(path => !expectedFiles.includes(path));
    const missing = expectedFiles.filter(path => !files.includes(path));
    throw new Error(
      `The staged package file set is not release-shaped. Unexpected: ${unexpected.join(', ') || 'none'}. Missing: ${missing.join(', ') || 'none'}.`,
    );
  }

  const forbiddenPath = files.find(path =>
    /(?:^|\/)(?:tests?|\.woml)(?:\/|$)|\.(?:node|db|sqlite|sqlite3|woml|env)$/iu.test(path),
  );
  if (forbiddenPath !== undefined) {
    throw new Error(`The staged package contains forbidden runtime data: ${forbiddenPath}`);
  }

  for (const path of publicPackageFiles) await assertNonempty(resolve(root, path));
  for (const path of publicSourceMapFiles) {
    const map = JSON.parse(await readFile(resolve(root, path), 'utf8')) as {
      readonly version?: number;
      readonly sources?: readonly string[];
    };
    if (map.version !== 3 || (map.sources?.length ?? 0) === 0) {
      throw new Error(`The source map ${path} is invalid.`);
    }
  }

  const cli = await readFile(resolve(root, 'dist/cli.js'), 'utf8');
  if (!cli.startsWith('#!/usr/bin/env bun')) {
    throw new Error('The staged CLI lost its Bun shebang.');
  }
  if (process.platform !== 'win32') {
    const mode = (await stat(resolve(root, 'dist/cli.js'))).mode;
    if ((mode & 0o111) === 0) throw new Error('The staged CLI is not executable.');
  }

  const scriptHost = await readFile(resolve(root, 'dist/script-host.js'), 'utf8');
  const customHost = await readFile(
    resolve(root, 'dist/custom-notification-provider-host.js'),
    'utf8',
  );
  if (!scriptHost.includes('script-host-worker.js')) {
    throw new Error('The packaged Script Host lost its worker path.');
  }
  if (!customHost.includes('custom-notification-provider-worker.js')) {
    throw new Error('The packaged custom provider host lost its worker path.');
  }

  const secretPattern = /(?:xox[baprs]-[A-Za-z0-9-]{20,}|ghp_[A-Za-z0-9]{20,}|sk-[A-Za-z0-9]{24,})/u;
  for (const path of files.filter(path => !path.endsWith('.map'))) {
    const source = await readFile(resolve(root, path), 'utf8');
    if (secretPattern.test(source)) {
      throw new Error(`The staged package appears to contain a credential in ${path}.`);
    }
  }
}

async function buildJavaScript(): Promise<void> {
  const result = Bun.spawnSync({
    cmd: [process.execPath, resolve(import.meta.dir, 'build-javascript.ts')],
    cwd: cliRoot,
    stdout: 'inherit',
    stderr: 'inherit',
  });
  if (result.exitCode !== 0) throw new Error('The JavaScript release build failed.');
}

async function inspectNpmPack(root: string): Promise<void> {
  const npmCache = resolve(tmpdir(), 'woml-npm-cache');
  await mkdir(npmCache, { recursive: true });
  const result = Bun.spawnSync({
    cmd: ['npm', 'pack', '--dry-run', '--json', '--ignore-scripts'],
    cwd: root,
    stdout: 'pipe',
    stderr: 'pipe',
    env: {
      ...process.env,
      npm_config_cache: npmCache,
      npm_config_update_notifier: 'false',
    },
  });
  if (result.exitCode !== 0) {
    throw new Error(`npm pack --dry-run failed: ${result.stderr.toString().trim()}`);
  }
  const report = JSON.parse(result.stdout.toString()) as Array<{
    readonly files?: ReadonlyArray<{ readonly path?: string }>;
  }>;
  const files = report[0]?.files?.flatMap(file =>
    typeof file.path === 'string' ? [file.path] : [],
  ) ?? [];
  if (!sameValues(files, expectedFiles)) {
    throw new Error(`npm pack would publish an unexpected file set: ${files.join(', ')}`);
  }
}

async function smokeCli(root: string): Promise<void> {
  for (const argument of ['--version', '--help']) {
    const result = Bun.spawnSync({
      cmd: [process.execPath, resolve(root, 'dist/cli.js'), argument],
      cwd: root,
      stdout: 'pipe',
      stderr: 'pipe',
    });
    if (result.exitCode !== 0) {
      throw new Error(`The staged CLI failed ${argument}: ${result.stderr.toString().trim()}`);
    }
  }
}

async function snapshot(root: string): Promise<string> {
  const hash = createHash('sha256');
  for (const path of await filesUnder(root)) {
    const metadata = await stat(resolve(root, path));
    hash.update(path);
    hash.update(String(metadata.mode & 0o777));
    hash.update(await readFile(resolve(root, path)));
  }
  return hash.digest('hex');
}

async function verifyPackagedReadmeContentShape(output: string): Promise<void> {
  const stagedReadmePath = resolve(output, 'README.md');
  const staged = await readFile(stagedReadmePath, 'utf8');
  const requiredMarkers: readonly { readonly label: string; readonly pattern: RegExp }[] = [
    { label: 'package name', pattern: /\bwoml-cli\b/u },
    { label: 'install command', pattern: /(?:bun\s+add|npm\s+(?:install|i)|pnpm\s+add)\s+(?:--global|-g|-g)?\s*woml-cli/u },
    { label: 'quick example or workflow sample', pattern: /<woml>/u },
    { label: 'commands section', pattern: /\bwoml\s+(?:check|run|inspect|list)\b/u },
  ];
  const missing = requiredMarkers
    .filter(marker => !marker.pattern.test(staged))
    .map(marker => marker.label);
  if (missing.length > 0) {
    throw new Error(
      `The packaged README at ${stagedReadmePath} is missing required content: ${missing.join(', ')}. ` +
        `Update woml-cli/README.md to include the package name, install command, a workflow example, and common commands.`,
    );
  }
}

async function prepareAndVerify(output: string): Promise<void> {
  await prepareMainPackage(output);
  await verifyMainPackage(output);
  await verifyPackagedReadmeContentShape(output);
  await inspectNpmPack(output);
  await smokeCli(output);
  process.stdout.write(`[package] staged and verified ${expectedFiles.length} files in ${output}\n`);
}

async function verifyReproducible(): Promise<void> {
  const temporaryRoot = await mkdtemp(resolve(tmpdir(), 'woml-release-'));
  try {
    const first = resolve(temporaryRoot, 'first');
    const second = resolve(temporaryRoot, 'second');
    await buildJavaScript();
    await prepareAndVerify(first);
    const firstHash = await snapshot(first);
    await buildJavaScript();
    await prepareAndVerify(second);
    const secondHash = await snapshot(second);
    if (firstHash !== secondHash) {
      throw new Error(`Two clean release builds differ: ${firstHash} != ${secondHash}`);
    }
    process.stdout.write(`[package] reproducible package ${firstHash.slice(0, 16)} (${expectedFiles.length} files)\n`);
  } finally {
    await rm(temporaryRoot, { recursive: true, force: true });
  }
}

function requiredOption(name: string): string {
  const index = process.argv.indexOf(name);
  const value = index < 0 ? undefined : process.argv[index + 1];
  if (value === undefined || value.length === 0) throw new Error(`Missing ${name}.`);
  return resolve(value);
}

async function pack(output: string, destination: string): Promise<void> {
  await buildJavaScript();
  await prepareAndVerify(output);
  await mkdir(destination, { recursive: true });
  const npmCache = resolve(tmpdir(), 'woml-npm-cache');
  await mkdir(npmCache, { recursive: true });
  const result = Bun.spawnSync({
    cmd: ['npm', 'pack', '--json', '--ignore-scripts', '--pack-destination', destination],
    cwd: output,
    stdout: 'pipe',
    stderr: 'pipe',
    env: {
      ...process.env,
      npm_config_cache: npmCache,
      npm_config_update_notifier: 'false',
    },
  });
  if (result.exitCode !== 0) {
    throw new Error(`npm pack failed: ${result.stderr.toString().trim()}`);
  }
  await chmod(resolve(output, 'dist/cli.js'), 0o755);
  process.stdout.write(`[package] wrote woml-cli@1.0.5 to ${destination}\n`);
}

async function main(): Promise<void> {
  const command = process.argv[2];
  if (command === 'verify') {
    await verifyMainPackage(requiredOption('--input'));
  } else if (command === 'stage') {
    await prepareAndVerify(requiredOption('--output'));
  } else if (command === 'reproducible') {
    await verifyReproducible();
  } else if (command === 'pack') {
    await pack(requiredOption('--output'), requiredOption('--destination'));
  } else {
    throw new Error(
      `Usage: ${basename(process.argv[1] ?? 'release-package.ts')} verify --input DIR | stage --output DIR | reproducible | pack --output DIR --destination DIR`,
    );
  }
}

if (import.meta.main) await main();
