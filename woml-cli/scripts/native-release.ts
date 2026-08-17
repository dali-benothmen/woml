#!/usr/bin/env bun

import {
  chmod,
  copyFile,
  cp,
  mkdir,
  readdir,
  readFile,
  rm,
  stat,
  writeFile,
} from 'node:fs/promises';
import { basename, resolve } from 'node:path';

import {
  nativePackageBinaryName,
  nativePackageName,
  type WomlNativeTarget,
  womlNativeTargets,
  womlNativeTargetSpecs,
} from '../src/native-platform';
import {
  publicJavaScriptFiles,
  publicPackageFiles,
  publicSourceMapFiles,
} from './release-contract';

const repositoryRoot = resolve(import.meta.dir, '../..');
const cliRoot = resolve(repositoryRoot, 'woml-cli');

interface PackageMetadata {
  readonly name: string;
  readonly version: string;
  readonly private?: boolean;
  readonly description: string;
  readonly license: string;
  readonly repository: Readonly<Record<string, string>>;
  readonly homepage: string;
  readonly bugs: Readonly<Record<string, string>>;
  readonly type: string;
  readonly bin: Readonly<Record<string, string>>;
  readonly publishConfig: Readonly<Record<string, string>>;
  readonly engines: Readonly<Record<string, string>>;
  readonly files?: readonly string[];
  readonly scripts?: Readonly<Record<string, string>>;
}

async function sourceMetadata(): Promise<PackageMetadata> {
  return JSON.parse(
    await readFile(resolve(cliRoot, 'package.json'), 'utf8'),
  ) as PackageMetadata;
}

function cargoPackageVersion(manifest: string, packageName: string): string | undefined {
  const packageSection = manifest.match(/\[package\]([\s\S]*?)(?=\n\[|$)/u)?.[1];
  if (
    packageSection?.match(/^name\s*=\s*"([^"]+)"\s*$/mu)?.[1] !== packageName
  ) {
    return undefined;
  }
  return packageSection.match(/^version\s*=\s*"([^"]+)"\s*$/mu)?.[1];
}

export async function verifySourceReleaseIdentity(
  metadata?: PackageMetadata,
): Promise<void> {
  const releaseMetadata = metadata ?? (await sourceMetadata());
  const frontend = JSON.parse(
    await readFile(resolve(repositoryRoot, 'woml/package.json'), 'utf8'),
  ) as PackageMetadata;
  const root = JSON.parse(
    await readFile(resolve(repositoryRoot, 'package.json'), 'utf8'),
  ) as PackageMetadata;
  const extension = JSON.parse(
    await readFile(resolve(repositoryRoot, 'woml-vscode/package.json'), 'utf8'),
  ) as PackageMetadata;
  const nativeManifest = await readFile(
    resolve(repositoryRoot, 'core/woml-native/Cargo.toml'),
    'utf8',
  );
  const engineManifest = await readFile(
    resolve(repositoryRoot, 'core/woml-engine/Cargo.toml'),
    'utf8',
  );
  const nativeVersion = cargoPackageVersion(nativeManifest, 'woml-native');
  const engineVersion = cargoPackageVersion(engineManifest, 'woml-engine');
  if (
    releaseMetadata.name !== 'woml' ||
    releaseMetadata.private !== false ||
    releaseMetadata.bin?.woml !== './dist/cli.js' ||
    frontend.name !== '@woml/compiler' ||
    frontend.private !== true ||
    root.name !== 'woml-repository' ||
    root.private !== true ||
    extension.name !== 'woml-language'
  ) {
    throw new Error(
      'Release package identities must be public woml, private @woml/compiler, private woml-repository, and woml-language.',
    );
  }
  if (
    frontend.version !== releaseMetadata.version ||
    root.version !== releaseMetadata.version ||
    extension.version !== releaseMetadata.version ||
    nativeVersion !== releaseMetadata.version ||
    engineVersion !== releaseMetadata.version
  ) {
    throw new Error(
      `Release versions must match: woml=${releaseMetadata.version}, @woml/compiler=${String(frontend.version)}, woml-repository=${String(root.version)}, woml-language=${String(extension.version)}, woml-native=${String(nativeVersion)}, woml-engine=${String(engineVersion)}.`,
    );
  }
}

function optionalNativeDependencies(version: string): Record<string, string> {
  return Object.fromEntries(
    womlNativeTargets.map(target => [nativePackageName(target), version]),
  );
}

function assertTarget(value: string | undefined): WomlNativeTarget {
  if (
    value === undefined ||
    !womlNativeTargets.includes(value as WomlNativeTarget)
  ) {
    throw new Error(
      `Unknown WOML native target ${JSON.stringify(value)}. Expected one of: ${womlNativeTargets.join(', ')}.`,
    );
  }
  return value as WomlNativeTarget;
}

async function resetDirectory(path: string): Promise<void> {
  await rm(path, { recursive: true, force: true });
  await mkdir(path, { recursive: true });
}

export async function createPlatformPackage(
  target: WomlNativeTarget,
  artifact: string,
  output: string,
): Promise<void> {
  const metadata = await sourceMetadata();
  await verifySourceReleaseIdentity(metadata);
  const spec = womlNativeTargetSpecs[target];
  const binary = nativePackageBinaryName(target);
  if ((await stat(artifact)).size === 0) {
    throw new Error(`Native artifact ${artifact} is empty.`);
  }
  await resetDirectory(output);
  await Promise.all([
    copyFile(artifact, resolve(output, binary)),
    copyFile(resolve(repositoryRoot, 'LICENSE'), resolve(output, 'LICENSE')),
  ]);
  await writeFile(
    resolve(output, 'README.md'),
    `# ${nativePackageName(target)}\n\nNative WOML execution engine for ${target}. This package is installed automatically by \`woml\`; do not install it directly.\n`,
  );
  await writeFile(
    resolve(output, 'package.json'),
    `${JSON.stringify(
      {
        name: nativePackageName(target),
        version: metadata.version,
        description: `WOML native execution engine for ${target}`,
        license: metadata.license,
        repository: metadata.repository,
        homepage: metadata.homepage,
        bugs: metadata.bugs,
        main: `./${binary}`,
        files: [binary, 'README.md', 'LICENSE'],
        os: [spec.os],
        cpu: [spec.cpu],
        ...(spec.libc === undefined ? {} : { libc: [spec.libc] }),
        engines: metadata.engines,
        publishConfig: metadata.publishConfig,
      },
      null,
      2,
    )}\n`,
  );
}

export async function prepareMainPackage(output: string): Promise<void> {
  const metadata = await sourceMetadata();
  await verifySourceReleaseIdentity(metadata);
  await resetDirectory(output);
  const sourceDist = resolve(cliRoot, 'dist');
  const outputDist = resolve(output, 'dist');
  await mkdir(outputDist, { recursive: true });
  for (const path of [...publicJavaScriptFiles, ...publicSourceMapFiles]) {
    const name = basename(path);
    await copyFile(resolve(sourceDist, name), resolve(outputDist, name));
    if (path.endsWith('.js')) await chmod(resolve(outputDist, name), 0o755);
  }
  await cp(resolve(cliRoot, 'slack'), resolve(output, 'slack'), {
    recursive: true,
  });
  await Promise.all([
    copyFile(resolve(cliRoot, 'README.md'), resolve(output, 'README.md')),
    copyFile(resolve(repositoryRoot, 'LICENSE'), resolve(output, 'LICENSE')),
  ]);
  await writeFile(
    resolve(output, 'package.json'),
    `${JSON.stringify(
      {
        name: metadata.name,
        version: metadata.version,
        description: metadata.description,
        license: metadata.license,
        repository: metadata.repository,
        homepage: metadata.homepage,
        bugs: metadata.bugs,
        type: metadata.type,
        bin: metadata.bin,
        files: publicPackageFiles,
        publishConfig: metadata.publishConfig,
        engines: metadata.engines,
        optionalDependencies: optionalNativeDependencies(metadata.version),
      },
      null,
      2,
    )}\n`,
  );
}

export function verifyReleaseTag(version: string, tag: string): void {
  if (tag !== `v${version}`) {
    throw new Error(
      `Release tag ${JSON.stringify(tag)} must exactly match woml version v${version}.`,
    );
  }
}

async function assertFile(path: string): Promise<void> {
  if ((await stat(path)).size === 0) throw new Error(`${path} is empty.`);
}

export async function verifyCollectedRelease(
  mainRoot: string,
  platformsRoot: string,
  tag: string,
): Promise<void> {
  const metadata = await sourceMetadata();
  await verifySourceReleaseIdentity(metadata);
  verifyReleaseTag(metadata.version, tag);
  const main = JSON.parse(
    await readFile(resolve(mainRoot, 'package.json'), 'utf8'),
  ) as PackageMetadata & {
    readonly optionalDependencies?: Readonly<Record<string, string>>;
  };
  if (
    main.name !== 'woml' ||
    main.version !== metadata.version ||
    JSON.stringify(main.optionalDependencies) !==
      JSON.stringify(optionalNativeDependencies(metadata.version))
  ) {
    throw new Error('The staged woml manifest has an invalid native package set.');
  }
  await Promise.all([
    assertFile(resolve(mainRoot, 'LICENSE')),
    assertFile(resolve(mainRoot, 'dist/cli.js')),
    assertFile(resolve(mainRoot, 'dist/script-host.js')),
  ]);
  const directories = (await readdir(platformsRoot, { withFileTypes: true }))
    .filter(entry => entry.isDirectory())
    .map(entry => resolve(platformsRoot, entry.name));
  const seen = new Set<string>();
  for (const directory of directories) {
    const manifest = JSON.parse(
      await readFile(resolve(directory, 'package.json'), 'utf8'),
    ) as {
      readonly name?: string;
      readonly version?: string;
      readonly main?: string;
    };
    const target = womlNativeTargets.find(
      candidate => nativePackageName(candidate) === manifest.name,
    );
    if (
      target === undefined ||
      manifest.version !== metadata.version ||
      manifest.main !== `./${nativePackageBinaryName(target)}`
    ) {
      throw new Error(`Invalid native package in ${directory}.`);
    }
    if (seen.has(target)) throw new Error(`Duplicate native package ${target}.`);
    seen.add(target);
    await Promise.all([
      assertFile(resolve(directory, nativePackageBinaryName(target))),
      assertFile(resolve(directory, 'LICENSE')),
    ]);
  }
  const missing = womlNativeTargets.filter(target => !seen.has(target));
  if (missing.length > 0) {
    throw new Error(`Collected release is missing: ${missing.join(', ')}.`);
  }
}

function option(name: string): string | undefined {
  const index = process.argv.indexOf(name);
  return index === -1 ? undefined : process.argv[index + 1];
}

function requiredOption(name: string): string {
  const value = option(name);
  if (value === undefined || value.length === 0) {
    throw new Error(`Missing required option ${name}.`);
  }
  return value;
}

async function main(): Promise<void> {
  const command = process.argv[2];
  if (command === 'verify-tag') {
    const metadata = await sourceMetadata();
    await verifySourceReleaseIdentity(metadata);
    verifyReleaseTag(metadata.version, requiredOption('--tag'));
    return;
  }
  if (command === 'package-platform') {
    await createPlatformPackage(
      assertTarget(requiredOption('--target')),
      resolve(requiredOption('--artifact')),
      resolve(requiredOption('--output')),
    );
    return;
  }
  if (command === 'prepare-main') {
    await prepareMainPackage(resolve(requiredOption('--output')));
    return;
  }
  if (command === 'verify-collected') {
    await verifyCollectedRelease(
      resolve(requiredOption('--main')),
      resolve(requiredOption('--platforms')),
      requiredOption('--tag'),
    );
    return;
  }
  throw new Error(
    `Usage: ${basename(process.argv[1] ?? 'native-release.ts')} verify-tag|package-platform|prepare-main|verify-collected`,
  );
}

if (import.meta.main) await main();
