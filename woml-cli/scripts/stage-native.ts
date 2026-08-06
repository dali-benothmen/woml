import { copyFile, mkdir, rm, stat } from 'node:fs/promises';
import { resolve } from 'node:path';

const packageRoot = resolve(import.meta.dir, '..');
const releaseDirectory = resolve(packageRoot, '../dist/target/release');

function rustLibraryName(): string {
  if (process.platform === 'win32') return 'core.dll';
  if (process.platform === 'darwin') return 'libcore.dylib';
  if (process.platform === 'linux') return 'libcore.so';
  throw new Error(`WOML does not support native builds for ${process.platform}.`);
}

const source = resolve(releaseDirectory, rustLibraryName());
const destinationDirectory = resolve(packageRoot, 'dist');
const destination = resolve(
  destinationDirectory,
  `woml-core.${process.platform}-${process.arch}.node`,
);

try {
  await stat(source);
} catch {
  throw new Error(
    `The WOML Rust library was not found at "${source}". Build core/Cargo.toml in release mode first.`,
  );
}

await mkdir(destinationDirectory, { recursive: true });
await rm(resolve(destinationDirectory, 'script-worker.ts'), { force: true });
await copyFile(source, destination);
