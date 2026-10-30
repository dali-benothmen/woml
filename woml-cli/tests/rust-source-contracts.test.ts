import { afterEach, expect, test } from 'bun:test';
import { mkdtemp, mkdir, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { nativeAdapterSourceViolations, readRustModuleSource, readRustSources } from '../scripts/rust-source';

const roots: string[] = [];
afterEach(async () => {
  for (const root of roots.splice(0)) await rm(root, { recursive: true, force: true });
});
async function fixture(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), 'woml-rust-source-'));
  roots.push(root);
  return root;
}

test('source guards scan a facade and all nested private Rust modules deterministically', async () => {
  const root = await fixture();
  await writeFile(join(root, 'event.rs'), 'pub use payloads::Event;');
  expect(await readRustModuleSource(join(root, 'event.rs'))).toBe('pub use payloads::Event;');
  await mkdir(join(root, 'event', 'payloads'), { recursive: true });
  await writeFile(join(root, 'event', 'payloads.rs'), 'mod details;');
  await writeFile(join(root, 'event', 'payloads', 'details.rs'), 'OperationalRecordV1');
  await writeFile(join(root, 'event', 'ignored.md'), 'not Rust');
  const source = await readRustModuleSource(join(root, 'event.rs'));
  expect(source).toContain('OperationalRecordV1');
  expect(source).not.toContain('not Rust');
  expect(await readRustModuleSource(join(root, 'event.rs'))).toBe(source);
  await expect(readRustModuleSource(join(root, 'absent.rs'))).rejects.toThrow();
});

test('native separation catches a prohibited dependency hidden in a child module', async () => {
  const root = await fixture();
  await writeFile(join(root, 'bridge.rs'), 'use woml_engine::RunEvent;');
  await mkdir(join(root, 'bridge', 'internal'), { recursive: true });
  await writeFile(join(root, 'bridge', 'internal', 'bad.rs'), 'use crate::models::Workflow;');
  const source = (await readRustSources(root)).join('\n');
  expect(source).toContain('use woml_engine::');
  expect(nativeAdapterSourceViolations(source)).toContain('crate-local legacy module');
  expect(nativeAdapterSourceViolations('use models::Workflow;')).toContain('legacy module models');
  expect(nativeAdapterSourceViolations('use woml_engine::RunEvent;')).toEqual([]);
  expect(nativeAdapterSourceViolations('use super::super::models::Workflow;')).toContain('legacy module models');
  expect(nativeAdapterSourceViolations(
    'use woml_engine::RunEvent; use super::configuration::runtime_options; use super::errors::native_execution_error;'
  )).toEqual([]);
});

test('a malformed owned module directory fails closed instead of skipping its source', async () => {
  const root = await fixture();
  await writeFile(join(root, 'event.rs'), 'mod payload;');
  await writeFile(join(root, 'event'), 'not a module directory');
  await expect(readRustModuleSource(join(root, 'event.rs'))).rejects.toThrow('directory');
});

test('source guards reject alternate Rust implementation paths in facades and nested children', async () => {
  const root = await fixture();
  const entry = join(root, 'event.rs');
  for (const declaration of [
    '#[path = "../outside.rs"] mod hidden;',
    '#[cfg_attr(unix, path = "../outside.rs")] mod hidden;',
    '#[cfg_attr(\n  unix,\n  path = "../outside.rs"\n)] mod hidden;',
    'include!("../outside.rs");',
  ]) {
    await writeFile(entry, declaration);
    await expect(readRustModuleSource(entry)).rejects.toThrow('owned module tree');
    await writeFile(entry, 'mod nested;');
    await mkdir(join(root, 'event'), { recursive: true });
    await writeFile(join(root, 'event', 'nested.rs'), declaration);
    await expect(readRustModuleSource(entry)).rejects.toThrow('owned module tree');
  }
  await writeFile(join(root, 'event', 'nested.rs'), 'const FIXTURE: &str = include_str!("reviewed.json");');
  expect(await readRustModuleSource(entry)).toContain('include_str!');
});

test('source guards reject symbolic module escapes', async () => {
  if (process.platform === 'win32') return; // Windows link privilege is not assumed.
  const root = await fixture();
  await writeFile(join(root, 'model.rs'), 'mod contract;');
  await writeFile(join(root, 'outside.rs'), 'hidden source');
  await mkdir(join(root, 'model'));
  await symlink(join(root, 'outside.rs'), join(root, 'model', 'contract.rs'));
  await expect(readRustModuleSource(join(root, 'model.rs'))).rejects.toThrow('symlink');
});
