import { mkdir, readdir, rm } from 'node:fs/promises';
import { resolve } from 'node:path';

import {
  createPlatformPackage,
  prepareMainPackage,
} from '../../scripts/native-release';
import {
  loadTestNativePackage,
  packReleaseArtifact,
  sealReleaseArtifact,
  verifyReleaseArtifact,
} from '../../scripts/release-artifact';
import {
  localNativeBinaryName,
  nativePackageName,
  nativeTargetForRuntime,
} from '../../src/native-platform';

const repositoryRoot = resolve(import.meta.dir, '../../..');
const cliRoot = resolve(repositoryRoot, 'woml-cli');
const candidateRoot = resolve(repositoryRoot, 'release/local-candidate');

export interface LocalReleaseCandidate {
  readonly mainArchive: string;
  readonly mainRoot: string;
  readonly nativeArchive: string;
  readonly nativeRoot: string;
  readonly nativePackage: string;
  readonly target: ReturnType<typeof nativeTargetForRuntime>;
}

let candidatePromise: Promise<LocalReleaseCandidate> | undefined;

async function archiveIn(root: string): Promise<string> {
  const archives = (await readdir(root))
    .filter(name => name.endsWith('.tgz'))
    .map(name => resolve(root, name));
  if (archives.length !== 1) {
    throw new Error(`Expected one npm archive in ${root}, found ${archives.length}.`);
  }
  return archives[0]!;
}

async function prepare(): Promise<LocalReleaseCandidate> {
  const target = nativeTargetForRuntime(
    process.platform,
    process.arch,
  );
  const mainRoot = resolve(candidateRoot, 'main');
  const nativeRoot = resolve(candidateRoot, `native-${target}`);
  // Test-only override lets resource-constrained local verification package
  // the freshly reviewed debug addon without overwriting the staged release
  // binary. CI uses the default freshly built release artifact.
  const localBinary = process.env.WOML_TEST_NATIVE_ARTIFACT === undefined ? resolve(
    cliRoot,
    'dist',
    localNativeBinaryName(process.platform, process.arch),
  ) : resolve(process.env.WOML_TEST_NATIVE_ARTIFACT);
  if (!(await Bun.file(localBinary).exists())) {
    throw new Error(
      `The local release candidate requires ${localBinary}. Run "bun run build" first.`,
    );
  }

  await rm(candidateRoot, { recursive: true, force: true });
  await mkdir(candidateRoot, { recursive: true });
  await prepareMainPackage(mainRoot);
  await createPlatformPackage(target, localBinary, nativeRoot);
  await loadTestNativePackage(nativeRoot, target);
  await Promise.all([
    packReleaseArtifact(mainRoot),
    packReleaseArtifact(nativeRoot),
  ]);
  await Promise.all([
    sealReleaseArtifact(mainRoot, 'main'),
    sealReleaseArtifact(nativeRoot, 'native', target),
  ]);
  await Promise.all([
    verifyReleaseArtifact(mainRoot),
    verifyReleaseArtifact(nativeRoot),
  ]);

  return {
    mainArchive: await archiveIn(mainRoot),
    mainRoot,
    nativeArchive: await archiveIn(nativeRoot),
    nativeRoot,
    nativePackage: nativePackageName(target),
    target,
  };
}

export function localReleaseCandidate(): Promise<LocalReleaseCandidate> {
  candidatePromise ??= prepare();
  return candidatePromise;
}

export async function installLocalReleaseCandidate(
  consumer: string,
  options: {
    readonly cache: string;
    readonly temporary?: string;
  },
): Promise<LocalReleaseCandidate> {
  const candidate = await localReleaseCandidate();
  await Promise.all([
    mkdir(consumer, { recursive: true }),
    mkdir(options.cache, { recursive: true }),
    ...(options.temporary === undefined
      ? []
      : [mkdir(options.temporary, { recursive: true })]),
  ]);
  const environment = {
    ...process.env,
    BUN_INSTALL_CACHE_DIR: options.cache,
    ...(options.temporary === undefined
      ? {}
      : { TMPDIR: options.temporary }),
  };
  for (const archive of [candidate.nativeArchive, candidate.mainArchive]) {
    const installed = Bun.spawnSync(
      [Bun.which('bun')!, 'add', archive, '--no-save'],
      {
        cwd: consumer,
        env: environment,
        stdout: 'pipe',
        stderr: 'pipe',
      },
    );
    if (installed.exitCode !== 0) {
      throw new Error(
        `Could not install the WOML release candidate:\n${installed.stdout.toString()}${installed.stderr.toString()}`,
      );
    }
  }
  return candidate;
}
