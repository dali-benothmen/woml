#!/usr/bin/env bun

import { chmod, mkdir, rm, stat } from 'node:fs/promises';
import { basename, resolve } from 'node:path';

import {
  publicJavaScriptFiles,
  publicSourceMapFiles,
} from './release-contract';

const cliRoot = resolve(import.meta.dir, '..');
const sourceRoot = resolve(cliRoot, 'src');
const outputRoot = resolve(cliRoot, 'dist');

const entrypoints = [
  'cli.ts',
  'script-host.ts',
  'script-host-worker.ts',
  'notification-provider-host.ts',
  'custom-notification-provider-host.ts',
  'custom-notification-provider-worker.ts',
].map(path => resolve(sourceRoot, path));

async function assertOutput(path: string): Promise<void> {
  if ((await stat(resolve(cliRoot, path))).size === 0) {
    throw new Error(`The JavaScript build produced an empty artifact: ${path}`);
  }
}

await mkdir(outputRoot, { recursive: true });
for (const path of [...publicJavaScriptFiles, ...publicSourceMapFiles]) {
  await rm(resolve(cliRoot, path), { force: true });
}

const result = await Bun.build({
  entrypoints,
  outdir: outputRoot,
  target: 'bun',
  format: 'esm',
  sourcemap: 'external',
  minify: true,
  naming: '[name].[ext]',
});

if (!result.success) {
  for (const log of result.logs) console.error(log);
  throw new Error('The WOML JavaScript build failed.');
}

for (const path of publicJavaScriptFiles) {
  await assertOutput(path);
  await chmod(resolve(cliRoot, path), 0o755);
}
for (const path of publicSourceMapFiles) await assertOutput(path);

const cli = await Bun.file(resolve(outputRoot, 'cli.js')).text();
if (!cli.startsWith('#!/usr/bin/env bun')) {
  throw new Error('The built WOML CLI lost its Bun shebang.');
}

process.stdout.write(
  `[build] created ${entrypoints.length} JavaScript entrypoints and source maps in ${basename(outputRoot)}\n`,
);
