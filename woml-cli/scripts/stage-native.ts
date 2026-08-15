import { copyFile, mkdir, rm, stat } from 'node:fs/promises';
import { resolve } from 'node:path';

const packageRoot = resolve(import.meta.dir, '..');
const releaseDirectory = resolve(packageRoot, '../dist/target/release');

export type WomlNativePlatform = 'win32' | 'darwin' | 'linux';
export type WomlNativeArchitecture = 'x64' | 'arm64';

export function rustLibraryName(platform: string): string {
  if (platform === 'win32') return 'woml_core.dll';
  if (platform === 'darwin') return 'libwoml_core.dylib';
  if (platform === 'linux') return 'libwoml_core.so';
  throw new Error(`WOML does not support native builds for ${platform}.`);
}

export function packagedAddonName(platform: string, architecture: string): string {
  if (!['win32', 'darwin', 'linux'].includes(platform)) {
    throw new Error(`WOML does not support native builds for ${platform}.`);
  }
  if (!['x64', 'arm64'].includes(architecture)) {
    throw new Error(`WOML does not support native builds for ${architecture}.`);
  }
  return `woml-core.${platform}-${architecture}.node`;
}

export async function stageNativeArtifact(
  platform = process.platform,
  architecture = process.arch,
): Promise<string> {
  const source = resolve(releaseDirectory, rustLibraryName(platform));
  const destinationDirectory = resolve(packageRoot, 'dist');
  const destination = resolve(
    destinationDirectory,
    packagedAddonName(platform, architecture),
  );

  try {
    await stat(source);
  } catch {
    throw new Error(
      `The WOML Rust library was not found at "${source}". Build core/woml-native/Cargo.toml in release mode first.`,
    );
  }

  await mkdir(destinationDirectory, { recursive: true });
  await rm(resolve(destinationDirectory, 'script-worker.ts'), { force: true });
  await copyFile(source, destination);
  return destination;
}

if (import.meta.main) {
  await stageNativeArtifact();
}
