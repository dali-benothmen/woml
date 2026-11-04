#!/usr/bin/env bun
// Explicit separate build step. Never called by a timing/portable contract test.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { copyFile, mkdir, mkdtemp, readFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';

const root = resolve(import.meta.dir, '../..');
const hash = (bytes: string | Uint8Array) => createHash('sha256').update(bytes).digest('hex');
function git(...args: string[]) {
  const result = Bun.spawnSync(['git', ...args], { cwd: root, stdout: 'pipe', stderr: 'pipe' });
  assert.equal(result.exitCode, 0, result.stderr.toString());
  return args[0] === 'diff' ? result.stdout.toString() : result.stdout.toString().trim();
}
async function main() {
  assert.ok(process.argv.length === 2 || (process.argv.length === 3 && process.argv[2] === '--worktree'), 'Only --worktree is supported; no implicit library/build overrides');
  const production = ['core', 'woml/src', 'woml-cli/src'];
  for (const name of ['CARGO_TARGET_DIR', 'CARGO_BUILD_TARGET', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS']) {
    assert.ok(!process.env[name], `Unset ${name}: this receipt uses the repository's standard native release build`);
  }
  assert.ok(!Object.keys(process.env).some(name => name.startsWith('CARGO_PROFILE_RELEASE_')), 'Custom release-profile environment needs its own reviewed experiment');
  const sourceCommit = git('rev-parse', 'HEAD');
  const sourceTreeSha256 = hash(git('ls-tree', '-r', 'HEAD', '--', ...production));
  const productionDiff = git('diff', 'HEAD', '--', ...production);
  assert.ok(!productionDiff || process.argv[2] === '--worktree', 'Commit production changes or explicitly use --worktree to retain their exact patch with the artifact');
  const check = () => {
    assert.equal(git('rev-parse', 'HEAD'), sourceCommit);
    assert.equal(git('diff', 'HEAD', '--', ...production), productionDiff, 'Production source changed during build');
    assert.equal(git('ls-files', '--others', '--exclude-standard', '--', ...production), '');
  };
  check();
  const command = ['cargo', 'build', '--offline', '-j', '1', '--locked', '--manifest-path', 'core/woml-native/Cargo.toml', '--lib', '--release'];
  const startedAt = new Date().toISOString();
  const build = Bun.spawn(command, { cwd: root, stdout: 'inherit', stderr: 'inherit' });
  assert.equal(await build.exited, 0, 'Optimized build failed'); check();
  const file = process.platform === 'win32' ? 'woml_core.dll' : process.platform === 'darwin' ? 'libwoml_core.dylib' : 'libwoml_core.so';
  const nativeSource = join(root, 'dist/target/release', file);
  const output = join(root, 'woml-cli/.woml/performance'); await mkdir(output, { recursive: true });
  const directory = await mkdtemp(join(output, 'startup-build-'));
  const path = join(directory, 'candidate.node'); await copyFile(nativeSource, path);
  await Bun.write(join(directory, 'source.patch'), productionDiff);
  const tool = (name: string) => {
    const result = Bun.spawnSync([name, '--version'], { stdout: 'pipe', stderr: 'pipe' });
    assert.equal(result.exitCode, 0); return result.stdout.toString().trim();
  };
  await Bun.write(join(directory, 'receipt.json'), JSON.stringify({ profile: 'woml.startup-separation-build/v1', buildMode: 'release',
    startedAt, completedAt: new Date().toISOString(), sourceCommit, sourceTreeSha256, productionDiffSha256: hash(productionDiff), productionDiffNormalization: 'raw',
    productionPatch: join(directory, 'source.patch'), command, rustc: tool('rustc'), cargo: tool('cargo'),
    native: { source: nativeSource, path, sha256: hash(await readFile(path)) },
    provenance: 'Local explicit Cargo release build with pinned HEAD and exact production patch checked before/after compilation. Untracked production files reject. Receipt is not independent signed build attestation.' }, null, 2));
  console.log(`Build receipt: ${join(directory, 'receipt.json')}`);
}
if (import.meta.main) await main();
