#!/usr/bin/env bun

import { createHash } from 'node:crypto';
import {
  mkdir,
  readFile,
  readdir,
  rm,
  stat,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, join, relative, resolve } from 'node:path';

import {
  nativePackageBinaryName,
  nativeTargetForRuntime,
  type WomlNativeTarget,
  womlNativeTargets,
} from '../src/native-platform';
import {
  expectedNativeExports,
  verifyNativeExports,
} from './verify-native-exports';

export const releaseArtifactManifestName = 'artifact-sha256.json';
export const nativeLoadReceiptName = 'native-load-test.json';

interface ArtifactFile {
  readonly path: string;
  readonly bytes: number;
  readonly sha256: string;
}

interface ArtifactManifest {
  readonly schemaVersion: 1;
  readonly kind: 'main' | 'native';
  readonly target?: WomlNativeTarget;
  readonly package: { readonly name: string; readonly version: string };
  readonly files: readonly ArtifactFile[];
}

interface NativeLoadReceipt {
  readonly schemaVersion: 1;
  readonly target: WomlNativeTarget;
  readonly binary: string;
  readonly binarySha256: string;
  readonly exports: readonly string[];
}

async function pathsUnder(root: string): Promise<string[]> {
  const files: string[] = [];
  const visit = async (directory: string): Promise<void> => {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) await visit(path);
      else if (entry.isFile()) {
        files.push(relative(root, path).replaceAll('\\', '/'));
      }
    }
  };
  await visit(root);
  return files.sort();
}

async function sha256(path: string): Promise<string> {
  return createHash('sha256').update(await readFile(path)).digest('hex');
}

async function packageIdentity(root: string): Promise<{
  readonly name: string;
  readonly version: string;
}> {
  const manifest = JSON.parse(
    await readFile(resolve(root, 'package.json'), 'utf8'),
  ) as { readonly name?: string; readonly version?: string };
  if (
    typeof manifest.name !== 'string' ||
    typeof manifest.version !== 'string'
  ) {
    throw new Error(`Release artifact ${root} has invalid package identity.`);
  }
  return { name: manifest.name, version: manifest.version };
}

function assertTarget(value: string | undefined): WomlNativeTarget {
  if (
    value === undefined ||
    !womlNativeTargets.includes(value as WomlNativeTarget)
  ) {
    throw new Error(`Unknown native target ${JSON.stringify(value)}.`);
  }
  return value as WomlNativeTarget;
}

export async function loadTestNativePackage(
  root: string,
  target: WomlNativeTarget,
): Promise<void> {
  const runtimeTarget = nativeTargetForRuntime(
    process.platform,
    process.arch,
  );
  if (runtimeTarget !== target) {
    throw new Error(
      `Native package ${target} must be load-tested on ${target}, not ${runtimeTarget}.`,
    );
  }
  const binary = nativePackageBinaryName(target);
  const binaryPath = resolve(root, binary);
  const count = await verifyNativeExports(binaryPath);
  if (count !== expectedNativeExports.length) {
    throw new Error(`Native package ${target} exposed ${count} exports.`);
  }
  const receipt: NativeLoadReceipt = {
    schemaVersion: 1,
    target,
    binary,
    binarySha256: await sha256(binaryPath),
    exports: [...expectedNativeExports].sort(),
  };
  await writeFile(
    resolve(root, nativeLoadReceiptName),
    `${JSON.stringify(receipt, null, 2)}\n`,
  );
}

export async function packReleaseArtifact(root: string): Promise<string> {
  for (const path of await pathsUnder(root)) {
    if (path.endsWith('.tgz')) await rm(resolve(root, path), { force: true });
  }
  const cache = resolve(tmpdir(), 'woml-npm-cache');
  await mkdir(cache, { recursive: true });
  const result = Bun.spawnSync({
    cmd: [
      'npm',
      'pack',
      '--json',
      '--ignore-scripts',
      '--pack-destination',
      root,
    ],
    cwd: root,
    stdout: 'pipe',
    stderr: 'pipe',
    env: {
      ...process.env,
      npm_config_cache: cache,
      npm_config_update_notifier: 'false',
    },
  });
  if (result.exitCode !== 0) {
    throw new Error(`npm pack failed: ${result.stderr.toString().trim()}`);
  }
  const report = JSON.parse(result.stdout.toString()) as Array<{
    readonly filename?: string;
  }>;
  const filename = report[0]?.filename;
  if (typeof filename !== 'string' || !filename.endsWith('.tgz')) {
    throw new Error('npm pack did not report one release archive.');
  }
  return filename;
}

export async function sealReleaseArtifact(
  root: string,
  kind: 'main' | 'native',
  target?: WomlNativeTarget,
): Promise<ArtifactManifest> {
  await rm(resolve(root, releaseArtifactManifestName), { force: true });
  if (kind === 'native' && target === undefined) {
    throw new Error('A native release artifact requires its exact target.');
  }
  if (kind === 'main' && target !== undefined) {
    throw new Error('The main release artifact cannot declare a native target.');
  }
  const files = await Promise.all(
    (await pathsUnder(root)).map(async path => ({
      path,
      bytes: (await stat(resolve(root, path))).size,
      sha256: await sha256(resolve(root, path)),
    })),
  );
  const manifest: ArtifactManifest = {
    schemaVersion: 1,
    kind,
    ...(target === undefined ? {} : { target }),
    package: await packageIdentity(root),
    files,
  };
  await writeFile(
    resolve(root, releaseArtifactManifestName),
    `${JSON.stringify(manifest, null, 2)}\n`,
  );
  return manifest;
}

export async function verifyReleaseArtifact(root: string): Promise<ArtifactManifest> {
  const manifest = JSON.parse(
    await readFile(resolve(root, releaseArtifactManifestName), 'utf8'),
  ) as ArtifactManifest;
  if (
    manifest.schemaVersion !== 1 ||
    !['main', 'native'].includes(manifest.kind) ||
    !Array.isArray(manifest.files)
  ) {
    throw new Error(`Release artifact ${root} has an invalid checksum manifest.`);
  }
  const actualPaths = (await pathsUnder(root)).filter(
    path => path !== releaseArtifactManifestName,
  );
  const expectedPaths = manifest.files.map(file => file.path);
  if (JSON.stringify(actualPaths) !== JSON.stringify(expectedPaths)) {
    throw new Error(`Release artifact ${root} changed after it was sealed.`);
  }
  for (const file of manifest.files) {
    const path = resolve(root, file.path);
    if ((await stat(path)).size !== file.bytes || (await sha256(path)) !== file.sha256) {
      throw new Error(`Release artifact checksum mismatch: ${file.path}.`);
    }
  }
  const identity = await packageIdentity(root);
  if (
    identity.name !== manifest.package.name ||
    identity.version !== manifest.package.version
  ) {
    throw new Error(`Release artifact ${root} changed package identity.`);
  }
  if (manifest.kind === 'native') {
    const target = assertTarget(manifest.target);
    const receipt = JSON.parse(
      await readFile(resolve(root, nativeLoadReceiptName), 'utf8'),
    ) as NativeLoadReceipt;
    const binary = nativePackageBinaryName(target);
    if (
      receipt.schemaVersion !== 1 ||
      receipt.target !== target ||
      receipt.binary !== binary ||
      receipt.binarySha256 !== (await sha256(resolve(root, binary))) ||
      JSON.stringify(receipt.exports) !==
        JSON.stringify([...expectedNativeExports].sort())
    ) {
      throw new Error(`Native load-test receipt is invalid for ${target}.`);
    }
  }
  if (manifest.files.filter(file => file.path.endsWith('.tgz')).length !== 1) {
    throw new Error(`Release artifact ${root} must contain exactly one npm archive.`);
  }
  return manifest;
}

function option(name: string): string | undefined {
  const index = process.argv.indexOf(name);
  return index < 0 ? undefined : process.argv[index + 1];
}

function required(name: string): string {
  const value = option(name);
  if (value === undefined || value.length === 0) throw new Error(`Missing ${name}.`);
  return value;
}

async function main(): Promise<void> {
  const command = process.argv[2];
  const root = resolve(required('--root'));
  if (command === 'load-native') {
    await loadTestNativePackage(root, assertTarget(required('--target')));
    process.stdout.write(`[artifact] native exports loaded successfully in ${root}\n`);
    return;
  }
  if (command === 'pack') {
    const archive = await packReleaseArtifact(root);
    process.stdout.write(`[artifact] created ${archive}\n`);
    return;
  }
  if (command === 'seal') {
    const kind = required('--kind');
    if (kind !== 'main' && kind !== 'native') throw new Error(`Invalid kind ${kind}.`);
    const manifest = await sealReleaseArtifact(
      root,
      kind,
      kind === 'native' ? assertTarget(required('--target')) : undefined,
    );
    process.stdout.write(`[artifact] sealed ${manifest.files.length} files in ${root}\n`);
    return;
  }
  if (command === 'verify') {
    const manifest = await verifyReleaseArtifact(root);
    process.stdout.write(`[artifact] verified ${manifest.kind} ${manifest.package.name}@${manifest.package.version}\n`);
    return;
  }
  throw new Error(
    `Usage: ${basename(process.argv[1] ?? 'release-artifact.ts')} load-native|pack|seal|verify --root DIR`,
  );
}

if (import.meta.main) await main();
