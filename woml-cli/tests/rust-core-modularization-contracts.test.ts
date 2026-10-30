import { expect, test } from 'bun:test';
import { createHash } from 'node:crypto';
import { readFile, readdir } from 'node:fs/promises';
import { resolve, relative } from 'node:path';
import { readRustModuleSource } from '../scripts/rust-source';
import { expectedNativeExports } from '../scripts/verify-native-exports';
import baseline from '../../docs/refactoring/rcm0-baseline.json';
import executionBaseline from '../../docs/refactoring/rcm2-baseline.json';
import runtimeBaseline from '../../docs/refactoring/rcm4-baseline.json';
import durableBaseline from '../../docs/refactoring/rcm6-baseline.json';

const root = resolve(import.meta.dir, '../..');
const hash = (value: string | Uint8Array): string => createHash('sha256').update(value).digest('hex');

test('contract facades preserve the frozen public type inventories', async () => {
  for (const [module, contract] of Object.entries({ ...baseline.contracts, ...executionBaseline.contracts, ...durableBaseline.contracts })) {
    const source = await readRustModuleSource(resolve(root, `core/woml-engine/src/${module}.rs`));
    const names = [...source.matchAll(/^pub (?:struct|enum|type) (\w+)/gm)].map(match => match[1]!).sort();
    expect(names).toEqual([...contract.publicTypes].sort());
    expect(new Set(names).size).toBe(names.length);
    const facade = await readFile(resolve(root, `core/woml-engine/src/${module}.rs`), 'utf8');
    expect(facade).not.toMatch(/pub\s+mod\s/);
    expect(facade).not.toMatch(/pub\s+use\s+[^;]*\*/);
  }
});

test('execution facades preserve their frozen public functions, constants, and traits', async () => {
  for (const [module, contract] of Object.entries({ ...executionBaseline.contracts, ...durableBaseline.contracts })) {
    const source = await readRustModuleSource(resolve(root, `core/woml-engine/src/${module}.rs`));
    const names = (pattern: RegExp): string[] => [...source.matchAll(pattern)].map(match => match[1]!).sort();
    expect(names(/^pub (?:async )?fn (\w+)/gm)).toEqual([...contract.publicFunctions].sort());
    expect(names(/^pub const (\w+)/gm)).toEqual([...contract.publicConstants].sort());
    expect(names(/^pub trait (\w+)/gm)).toEqual([...contract.publicTraits].sort());
  }
});

test('contract extraction does not change native exports or hide integration targets', async () => {
  const exports: string[] = [...expectedNativeExports];
  expect(exports.sort()).toEqual(baseline.nativeExports);
  const targets = (await readdir(resolve(root, 'core/woml-engine/tests')))
    .filter(name => name.endsWith('.rs')).map(name => name.slice(0, -3));
  expect(baseline.integrationTargets.filter(name => !targets.includes(name))).toEqual([]);
});

test('runtime extraction preserves intentional boxed invocation and continuation boundaries', async () => {
  const source = await readRustModuleSource(resolve(root, 'core/woml-engine/src/runtime.rs'));
  const code = source.replace(/\/\/[^\n]*|\/\*[\s\S]*?\*\/|r#*"[\s\S]*?"#*|"(?:\\.|[^"\\])*"/g, ' ');
  const callees = [...code.matchAll(/\bBox::pin\s*\(\s*([\w:.]+)/g)].map(match => match[1]!).sort();
  // Complements ordinary-stack runtime tests; counting call sites alone is
  // not proof of cancellation, ownership, stack safety, or equivalent futures.
  expect(callees).toEqual(runtimeBaseline.boxedCallees);
});

test('durable extraction preserves the exact frozen schema and migration DDL literals', async () => {
  const source = (await readRustModuleSource(resolve(root, 'core/woml-engine/src/durable.rs')))
    .replaceAll('\r\n', '\n');
  const ddl = [...source.matchAll(/^(?:pub(?:\([^)]*\))? )?const ((?:CREATE|MIGRATE)_\w+): &str = (r#"[\s\S]*?"#);/gm)]
    .map(match => ({ name: match[1]!, sha256: hash(match[2]!) }))
    .sort((left, right) => left.name.localeCompare(right.name));
  expect(ddl).toEqual(durableBaseline.ddl);
});

test('the reviewed text fixture tree retains its canonical LF bytes', async () => {
  async function files(directory: string): Promise<string[]> {
    const result: string[] = [];
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = resolve(directory, entry.name);
      if (entry.isDirectory()) result.push(...await files(path));
      else if (entry.isFile()) result.push(path);
    }
    return result.sort();
  }
  const paths = await files(resolve(root, baseline.fixtureTree.root));
  // Hash canonical slash-separated relative paths so Windows and Unix agree.
  const entries = await Promise.all(paths.map(async path => {
    const name = relative(root, path).replaceAll('\\', '/');
    // Git may check text out as CRLF on Windows; freeze content, not autocrlf.
    const text = (await readFile(path, 'utf8')).replaceAll('\r\n', '\n');
    return `${name}\0${hash(text)}\n`;
  }));
  expect(paths.length).toBe(baseline.fixtureTree.files);
  expect(hash(entries.sort().join(''))).toBe(baseline.fixtureTree.sha256);
});
