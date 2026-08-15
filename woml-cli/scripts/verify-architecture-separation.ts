#!/usr/bin/env bun

import {
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  rm,
  stat,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { extname, join, relative, resolve } from 'node:path';

import {
  expectedNativeExports,
  verifyNativeExports,
} from './verify-native-exports';

const sourceExtensions = new Set(['.js', '.cjs', '.mjs', '.ts', '.tsx']);
const ignoredDirectories = new Set(['dist', 'node_modules', 'target']);
const staticImportPattern =
  /\b(?:import|export)\s+(?:type\s+)?(?:[^'";]*?\sfrom\s*)?['"]([^'"]+)['"]/gu;
const callableImportPattern =
  /\b(?:import|require)\s*\(\s*['"]([^'"]+)['"]\s*\)/gu;

export function importSpecifiers(source: string): readonly string[] {
  return [staticImportPattern, callableImportPattern].flatMap(pattern =>
    [...source.matchAll(pattern)].map(match => match[1]!)
  );
}

export function isLegacySdkSpecifier(specifier: string): boolean {
  const normalized = specifier.replaceAll('\\', '/');
  return (
    normalized === 'cronflow' ||
    normalized.startsWith('@cronflow/') ||
    normalized.split('/').includes('sdk')
  );
}

export function cronflowRuntimeDependencies(
  manifest: Readonly<Record<string, unknown>>
): readonly string[] {
  return ['dependencies', 'optionalDependencies', 'peerDependencies'].flatMap(
    field => {
      const value = manifest[field];
      if (value === null || typeof value !== 'object' || Array.isArray(value)) {
        return [];
      }
      return Object.entries(value)
        .filter(([name, version]) => {
          const normalizedVersion =
            typeof version === 'string' ? version.replaceAll('\\', '/') : '';
          return (
            name === 'cronflow' ||
            name.startsWith('@cronflow/') ||
            normalizedVersion.split('/').includes('sdk')
          );
        })
        .map(([name]) => name);
    }
  );
}

async function filesBelow(directory: string): Promise<readonly string[]> {
  const output: string[] = [];
  const visit = async (current: string): Promise<void> => {
    for (const entry of await readdir(current, { withFileTypes: true })) {
      if (entry.isDirectory() && ignoredDirectories.has(entry.name)) continue;
      const path = join(current, entry.name);
      if (entry.isDirectory()) await visit(path);
      else if (sourceExtensions.has(extname(entry.name))) output.push(path);
    }
  };
  await visit(directory);
  return output.sort();
}

async function assertNoSdkImports(root: string): Promise<number> {
  const sourceRoots = [
    resolve(root, 'woml/src'),
    resolve(root, 'woml-cli/src'),
    resolve(root, 'woml-cli/scripts'),
  ];
  let scanned = 0;
  for (const sourceRoot of sourceRoots) {
    for (const path of await filesBelow(sourceRoot)) {
      scanned += 1;
      const source = await readFile(path, 'utf8');
      const forbidden = importSpecifiers(source).filter(isLegacySdkSpecifier);
      if (forbidden.length > 0) {
        throw new Error(
          `${relative(root, path)} imports the legacy SDK: ${forbidden.join(', ')}`
        );
      }
    }
  }
  for (const manifestPath of ['woml/package.json', 'woml-cli/package.json']) {
    const manifest = JSON.parse(
      await readFile(resolve(root, manifestPath), 'utf8')
    ) as Readonly<Record<string, unknown>>;
    const forbidden = cronflowRuntimeDependencies(manifest);
    if (forbidden.length > 0) {
      throw new Error(
        `${manifestPath} declares a legacy runtime dependency: ${forbidden.join(', ')}`
      );
    }
  }
  return scanned;
}

async function assertNativeSourceSeparation(root: string): Promise<void> {
  const manifest = await readFile(
    resolve(root, 'core/woml-native/Cargo.toml'),
    'utf8'
  );
  const localDependencies = manifest
    .split(/\r?\n/u)
    .filter(line => line.includes('path ='));
  if (
    localDependencies.length !== 1 ||
    localDependencies[0]?.includes('woml-engine') !== true
  ) {
    throw new Error(
      `woml-native has unexpected local dependencies: ${localDependencies.join(', ')}`
    );
  }

  const adapter = await readFile(
    resolve(root, 'core/woml-native/src/bridge.rs'),
    'utf8'
  );
  const legacyModules = [
    'bridge',
    'condition_evaluator',
    'config',
    'context',
    'database',
    'dispatcher',
    'error',
    'job',
    'models',
    'state',
    'step_orchestrator',
    'trigger_executor',
    'triggers',
    'webhook_server',
    'workflow_state_machine',
  ];
  if (adapter.includes('crate::')) {
    throw new Error('The WOML adapter imports a crate-local legacy module.');
  }
  for (const module of legacyModules) {
    if (adapter.includes(`use ${module}::`)) {
      throw new Error(`The WOML adapter imports legacy module ${module}.`);
    }
  }
  if (!adapter.includes('use woml_engine::')) {
    throw new Error('The WOML adapter does not import the authoritative engine.');
  }
}

async function assertCliNativeContract(root: string): Promise<void> {
  const executor = await readFile(
    resolve(root, 'woml-cli/src/rust-executor.ts'),
    'utf8'
  );
  for (const name of expectedNativeExports) {
    if (!new RegExp(`readonly\\s+${name}\\s*\\??\\s*:`).test(executor)) {
      throw new Error(
        `NativeCore does not declare required native export ${name}.`
      );
    }
  }
  await verifyNativeExports();
}

async function assertCleanPackage(root: string): Promise<void> {
  const packageRoot = resolve(root, 'woml-cli');
  const temporaryDirectory = await mkdtemp(
    join(tmpdir(), 'woml-architecture-separation-')
  );
  const archives = join(temporaryDirectory, 'archives');
  const consumer = join(temporaryDirectory, 'consumer');
  const cache = join(temporaryDirectory, 'cache');
  await Promise.all(
    [archives, consumer, cache].map(path => mkdir(path, { recursive: true }))
  );

  try {
    await writeFile(
      join(consumer, 'package.json'),
      JSON.stringify({ name: 'woml-separation-consumer', private: true })
    );
    const packed = Bun.spawnSync(
      [
        Bun.which('bun')!,
        'pm',
        'pack',
        '--ignore-scripts',
        '--destination',
        archives,
      ],
      { cwd: packageRoot, stdout: 'pipe', stderr: 'pipe' }
    );
    if (packed.exitCode !== 0) {
      throw new Error(
        `Could not pack WOML CLI:\n${packed.stdout.toString()}${packed.stderr.toString()}`
      );
    }
    const archiveName = (await readdir(archives)).find(name =>
      name.endsWith('.tgz')
    );
    if (archiveName === undefined) {
      throw new Error('WOML CLI pack did not create an archive.');
    }
    const installed = Bun.spawnSync(
      [
        Bun.which('bun')!,
        'add',
        join(archives, archiveName),
        '--no-save',
      ],
      {
        cwd: consumer,
        env: { ...process.env, BUN_INSTALL_CACHE_DIR: cache },
        stdout: 'pipe',
        stderr: 'pipe',
      }
    );
    if (installed.exitCode !== 0) {
      throw new Error(
        `Could not install packed WOML CLI:\n${installed.stdout.toString()}${installed.stderr.toString()}`
      );
    }

    const installedRoot = join(consumer, 'node_modules/woml-cli');
    const installedManifest = JSON.parse(
      await readFile(join(installedRoot, 'package.json'), 'utf8')
    ) as Readonly<Record<string, unknown>>;
    const forbidden = cronflowRuntimeDependencies(installedManifest);
    if (forbidden.length > 0) {
      throw new Error(
        `The clean WOML package requires legacy packages: ${forbidden.join(', ')}`
      );
    }
    let installedLegacyPackages = true;
    try {
      await stat(join(consumer, 'node_modules/@cronflow'));
    } catch (error) {
      if (
        typeof error === 'object' &&
        error !== null &&
        'code' in error &&
        error.code === 'ENOENT'
      ) {
        installedLegacyPackages = false;
      } else {
        throw error;
      }
    }
    if (installedLegacyPackages) {
      throw new Error('The clean WOML install contains an @cronflow package.');
    }
    const version = Bun.spawnSync(
      [join(consumer, 'node_modules/.bin/woml'), '--version'],
      { cwd: consumer, stdout: 'pipe', stderr: 'pipe' }
    );
    if (version.exitCode !== 0 || !/^woml \d+\.\d+\.\d+\s*$/u.test(version.stdout.toString())) {
      throw new Error(
        `The clean WOML command is not executable:\n${version.stdout.toString()}${version.stderr.toString()}`
      );
    }
  } finally {
    await rm(temporaryDirectory, { recursive: true, force: true });
  }
}

export async function verifyArchitectureSeparation(): Promise<void> {
  const root = resolve(import.meta.dir, '../..');
  const scanned = await assertNoSdkImports(root);
  await assertNativeSourceSeparation(root);
  await assertCliNativeContract(root);
  await assertCleanPackage(root);
  console.log(
    `WOML architecture separation verified: ${scanned} frontend/CLI source files, one engine-only native dependency, ${expectedNativeExports.length} required addon exports, and a clean package with no @cronflow runtime dependency.`
  );
}

if (import.meta.main) {
  await verifyArchitectureSeparation();
}
