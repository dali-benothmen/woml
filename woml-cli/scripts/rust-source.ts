import { lstat, readdir, readFile } from 'node:fs/promises';
import { basename, dirname, extname, join, resolve } from 'node:path';

/** Read every Rust file in an owned directory, including nested private modules. */
export async function readRustSources(directory: string): Promise<readonly string[]> {
  const files: string[] = [];
  async function visit(path: string): Promise<void> {
    const info = await lstat(path);
    if (info.isSymbolicLink()) throw new Error(`Rust source must not be a symlink: ${path}`);
    if (info.isDirectory()) {
      for (const name of (await readdir(path)).sort()) await visit(join(path, name));
    } else if (info.isFile() && extname(path) === '.rs') files.push(path);
  }
  await visit(directory);
  return Promise.all(files.map(file => readFile(file, 'utf8')));
}

/** Keep source guards covering a stable facade AND its same-named implementation tree. */
export async function readRustModuleSource(entry: string): Promise<string> {
  const path = resolve(entry);
  const info = await lstat(path);
  if (!info.isFile() || info.isSymbolicLink() || extname(path) !== '.rs') {
    throw new Error(`Expected a regular Rust facade: ${path}`);
  }
  const sources = [await readFile(path, 'utf8')];
  const children = join(dirname(path), basename(path, '.rs'));
  try {
    const directory = await lstat(children);
    if (!directory.isDirectory() || directory.isSymbolicLink()) {
      throw new Error(`Expected an owned Rust module directory: ${children}`);
    }
  } catch (error) {
    if (typeof error === 'object' && error !== null && 'code' in error && error.code === 'ENOENT') {
      return sources.join('\n');
    }
    throw error;
  }
  sources.push(...await readRustSources(children));
  return sources.join('\n');
}

export function nativeAdapterSourceViolations(source: string): readonly string[] {
  const violations: string[] = [];
  if (source.includes('crate::')) violations.push('crate-local legacy module');
  for (const module of [
    'bridge', 'condition_evaluator', 'config', 'context', 'database', 'dispatcher',
    'error', 'job', 'models', 'state', 'step_orchestrator', 'trigger_executor',
    'triggers', 'webhook_server', 'workflow_state_machine',
  ]) {
    if (source.includes(`use ${module}::`)) violations.push(`legacy module ${module}`);
  }
  return violations;
}
