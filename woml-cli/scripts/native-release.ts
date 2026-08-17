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
  packedPackageFiles,
  publicJavaScriptFiles,
  publicPackageFiles,
  publicSourceMapFiles,
} from './release-contract';
import {
  nativeLoadReceiptName,
  verifyReleaseArtifact,
} from './release-artifact';

const repositoryRoot = resolve(import.meta.dir, '../..');
const cliRoot = resolve(repositoryRoot, 'woml-cli');

interface PackageMetadata {
  readonly name: string;
  readonly version: string;
  readonly private?: boolean;
  readonly description: string;
  readonly author: string;
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
  readonly main?: string;
  readonly os?: readonly string[];
  readonly cpu?: readonly string[];
  readonly libc?: readonly string[];
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
    releaseMetadata.name !== '@woml-org/woml' ||
    releaseMetadata.private !== false ||
    releaseMetadata.bin?.woml !== './dist/cli.js' ||
    frontend.name !== '@woml/compiler' ||
    frontend.private !== true ||
    root.name !== 'woml-repository' ||
    root.private !== true ||
    extension.name !== 'woml-language'
  ) {
    throw new Error(
      'Release package identities must be public @woml-org/woml, private @woml/compiler, private woml-repository, and woml-language.',
    );
  }
  if (
    releaseMetadata.author !== 'Mohamed Ali Ben Othmen' ||
    frontend.author !== releaseMetadata.author ||
    root.author !== releaseMetadata.author ||
    extension.author !== releaseMetadata.author ||
    releaseMetadata.license !== 'Apache-2.0' ||
    frontend.license !== releaseMetadata.license ||
    root.license !== releaseMetadata.license ||
    extension.license !== releaseMetadata.license ||
    !nativeManifest.includes('license = "Apache-2.0"') ||
    !engineManifest.includes('license = "Apache-2.0"') ||
    !nativeManifest.includes('repository = "https://github.com/dali-benothmen/woml"') ||
    !engineManifest.includes('repository = "https://github.com/dali-benothmen/woml"')
  ) {
    throw new Error('Release author, license, or repository metadata is inconsistent.');
  }
  if (
    frontend.version !== releaseMetadata.version ||
    root.version !== releaseMetadata.version ||
    extension.version !== releaseMetadata.version ||
    nativeVersion !== releaseMetadata.version ||
    engineVersion !== releaseMetadata.version
  ) {
    throw new Error(
      `Release versions must match: @woml-org/woml=${releaseMetadata.version}, @woml/compiler=${String(frontend.version)}, woml-repository=${String(root.version)}, woml-language=${String(extension.version)}, woml-native=${String(nativeVersion)}, woml-engine=${String(engineVersion)}.`,
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
    copyFile(resolve(repositoryRoot, 'NOTICE.md'), resolve(output, 'NOTICE.md')),
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
        author: metadata.author,
        license: metadata.license,
        repository: metadata.repository,
        homepage: metadata.homepage,
        bugs: metadata.bugs,
        main: `./${binary}`,
        files: [binary, 'README.md', 'LICENSE', 'NOTICE.md'],
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
    copyFile(resolve(repositoryRoot, 'NOTICE.md'), resolve(output, 'NOTICE.md')),
  ]);
  await writeFile(
    resolve(output, 'package.json'),
    `${JSON.stringify(
      {
        name: metadata.name,
        version: metadata.version,
        description: metadata.description,
        author: metadata.author,
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

function assertArtifactFiles(
  label: string,
  actual: readonly { readonly path: string }[],
  expected: readonly string[],
): void {
  const archives = actual.filter(file => file.path.endsWith('.tgz'));
  const files = actual
    .filter(file => !file.path.endsWith('.tgz'))
    .map(file => file.path)
    .sort();
  if (
    archives.length !== 1 ||
    JSON.stringify(files) !== JSON.stringify([...expected].sort())
  ) {
    throw new Error(`${label} has an invalid sealed file inventory.`);
  }
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
    main.name !== '@woml-org/woml' ||
    main.version !== metadata.version ||
    JSON.stringify(main.optionalDependencies) !==
      JSON.stringify(optionalNativeDependencies(metadata.version))
  ) {
    throw new Error('The staged @woml-org/woml manifest has an invalid native package set.');
  }
  const mainArtifact = await verifyReleaseArtifact(mainRoot);
  if (
    mainArtifact.kind !== 'main' ||
    mainArtifact.target !== undefined ||
    !main.files ||
    JSON.stringify(main.files) !== JSON.stringify(publicPackageFiles) ||
    mainArtifact.files.some(file => file.path.endsWith('.node'))
  ) {
    throw new Error('The staged @woml-org/woml artifact is not the sealed portable package.');
  }
  assertArtifactFiles('The staged @woml-org/woml artifact', mainArtifact.files, [
    ...packedPackageFiles,
  ]);
  await Promise.all([
    assertFile(resolve(mainRoot, 'LICENSE')),
    assertFile(resolve(mainRoot, 'NOTICE.md')),
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
    ) as PackageMetadata;
    const target = womlNativeTargets.find(
      candidate => nativePackageName(candidate) === manifest.name,
    );
    const spec = target === undefined ? undefined : womlNativeTargetSpecs[target];
    if (
      target === undefined ||
      spec === undefined ||
      manifest.version !== metadata.version ||
      manifest.author !== metadata.author ||
      manifest.license !== metadata.license ||
      manifest.main !== `./${nativePackageBinaryName(target)}` ||
      JSON.stringify(manifest.files) !==
        JSON.stringify([nativePackageBinaryName(target), 'README.md', 'LICENSE', 'NOTICE.md']) ||
      JSON.stringify(manifest.os) !== JSON.stringify([spec.os]) ||
      JSON.stringify(manifest.cpu) !== JSON.stringify([spec.cpu]) ||
      JSON.stringify(manifest.libc) !==
        JSON.stringify(spec.libc === undefined ? undefined : [spec.libc])
    ) {
      throw new Error(`Invalid native package in ${directory}.`);
    }
    if (seen.has(target)) throw new Error(`Duplicate native package ${target}.`);
    seen.add(target);
    const artifact = await verifyReleaseArtifact(directory);
    if (artifact.kind !== 'native' || artifact.target !== target) {
      throw new Error(`Native artifact seal does not match ${target}.`);
    }
    assertArtifactFiles(`Native artifact ${target}`, artifact.files, [
      nativePackageBinaryName(target),
      'LICENSE',
      'NOTICE.md',
      'README.md',
      'package.json',
      nativeLoadReceiptName,
    ]);
    await Promise.all([
      assertFile(resolve(directory, nativePackageBinaryName(target))),
      assertFile(resolve(directory, 'LICENSE')),
      assertFile(resolve(directory, 'NOTICE.md')),
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
