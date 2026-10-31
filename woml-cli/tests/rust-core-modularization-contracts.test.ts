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
import transactionBaseline from '../../docs/refactoring/rcm8-baseline.json';
import capabilityArtifactBaseline from '../../docs/refactoring/rcm10-baseline.json';
import featureBridgeBaseline from '../../docs/refactoring/rcm12-baseline.json';
import fixtureBaseline from '../../docs/refactoring/rcm14-fixtures-baseline.json';
import compatibilityRepairs from '../../docs/refactoring/post-refactor-compatibility-repairs.json';

const root = resolve(import.meta.dir, '../..');
const hash = (value: string | Uint8Array): string => createHash('sha256').update(value).digest('hex');

function declarationFingerprints(source: string): { name: string; sha256: string }[] {
  const clean = source.replace(/^(?:pub(?:\([^)]*\))? )?use [\s\S]*?;\n/gm, '');
  const metadata = (text: string): boolean => !text.replace(/\/\/[^\n]*/g, '')
    .replace(/#\[[\s\S]*?\]/g, '').trim();
  const canonical = (text: string): string => {
    const tokens = (text.replace(/\bpub\((?:super|in crate::[a-z_]+)\)\s*/g, '')
      .match(/r#*"[\s\S]*?"#*|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])'|\/\/[^\n]*|\/\*[\s\S]*?\*\/|[^\s]/g) ?? [])
      .filter(token => !token.startsWith('//') && !token.startsWith('/*'));
    return tokens.filter((token, index) => !(token === ',' && [')', ']', '}'].includes(tokens[index + 1]!))).join('\0');
  };
  const heads = [...clean.matchAll(/^(?:(?:pub(?:\([^)]*\))?) )?(?:(?:async|const) )?(struct|enum|trait|type|const|static|fn|mod) (\w+)|^(?:unsafe )?impl[^\n]*/gm)];
  const starts = heads.map(match => {
    const start = clean.lastIndexOf('\n\n', match.index) + 2;
    return metadata(clean.slice(start, match.index)) ? start : match.index!;
  });
  return heads.flatMap((head, index) => {
    if (head[1] === 'mod') return [];
    const text = clean.slice(starts[index], starts[index + 1] ?? clean.length).trimEnd();
    if (head[1] !== undefined) return [{ name: `${head[1]} ${head[2]}`, sha256: hash(canonical(text)) }];
    const owner = text.slice(0, text.indexOf('{')).trim();
    const methods = [...text.matchAll(/^  (?:pub(?:\([^)]*\))? )?(?:(?:async|const) )?fn (\w+)/gm)];
    // Scoped visibility is normalized for moved inherent helpers below, but
    // it is illegal on trait methods (including cfg-gated Windows Drop).
    // Reject it before normalization can conceal the extraction mistake.
    if (/\bfor\s+[A-Za-z_:]/u.test(owner)
      && methods.some(method => method[0].trimStart().startsWith('pub'))) {
      throw new Error(`Visibility qualifiers are not permitted in trait implementations: ${owner}`);
    }
    if (methods.length === 0) return [{ name: owner, sha256: hash(canonical(text)) }];
    const bounds = methods.map(match => {
      const start = Math.max(text.lastIndexOf('\n\n', match.index) + 2, text.indexOf('\n') + 1);
      return metadata(text.slice(start, match.index)) ? start : match.index!;
    });
    return methods.map((method, position) => ({ name: `${owner}::${method[1]}`,
      sha256: hash(canonical(text.slice(bounds[position], bounds[position + 1] ?? text.lastIndexOf('\n}')))) }));
  });
}

test('declaration guards reject trait-method visibility before fingerprint normalization', () => {
  const valid = 'impl Drop for WindowsJob {\n  fn drop(&mut self) {}\n}\n';
  expect(declarationFingerprints(valid)).toHaveLength(1);
  for (const visibility of ['pub', 'pub(crate)', 'pub(super)', 'pub(in crate::process)']) {
    expect(() => declarationFingerprints(valid.replace('  fn drop', `  ${visibility} fn drop`)))
      .toThrow('Visibility qualifiers are not permitted in trait implementations');
  }
  expect(declarationFingerprints('impl WindowsJob {\n  pub(in crate::process) fn attach() {}\n}\n')).toHaveLength(1);
});

test('extracted engine and bridge families preserve every ordered production declaration', async () => {
  async function productionFiles(directory: string): Promise<string[]> {
    const paths: string[] = [];
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      if (entry.name === 'tests' || entry.name === 'tests.rs') continue;
      const path = resolve(directory, entry.name);
      if (entry.isDirectory()) paths.push(...await productionFiles(path));
      else if (entry.isFile() && entry.name.endsWith('.rs')) paths.push(path);
      else if (entry.isSymbolicLink()) throw new Error(`Unsafe owned Rust path: ${path}`);
    }
    return paths;
  }
  const sort = (items: { name: string; sha256: string }[]): string[] =>
    items.map(item => `${item.name}:${item.sha256}`).sort();
  for (const [module, contract] of Object.entries({ ...capabilityArtifactBaseline.contracts, ...featureBridgeBaseline.contracts })) {
    const entry = resolve(root, 'path' in contract ? contract.path : `core/woml-engine/src/${module}.rs`);
    // At intermediate checkpoints an untouched facade may not have a child
    // directory yet. Frozen inline test modules are checked separately.
    const files = [entry, ...await productionFiles(entry.slice(0, -3)).catch(error => {
      if (error?.code === 'ENOENT') return [];
      throw error;
    })];
    const actual = (await Promise.all(files.map(path => readFile(path, 'utf8'))))
      .flatMap(source => declarationFingerprints(source.replaceAll('\r\n', '\n')));
    expect(sort(actual)).toEqual(sort(contract.declarations));
  }
});

test('the reviewed cohesive families remain unchanged during RCM12 and RCM13', async () => {
  for (const retained of featureBridgeBaseline.retained) {
    expect(hash((await readFile(resolve(root, retained.path), 'utf8')).replaceAll('\r\n', '\n')))
      .toBe(retained.sha256);
  }
  const bridge = await readFile(resolve(root, featureBridgeBaseline.contracts.bridge.path), 'utf8');
  const names = [...bridge.matchAll(/^pub (?:async )?fn (\w+)/gm)].map(match => match[1]!).sort();
  expect(names).toEqual(featureBridgeBaseline.contracts.bridge.publicFunctions);
  expect([...bridge.matchAll(/^#\[napi(?:\(|\])/gm)].length).toBe(names.length);
});

test('relocated process, artifact, and ingress tests retain assertions, fixtures, and platform cases', async () => {
  async function testSources(path: string): Promise<string[]> {
    if (path.endsWith('.rs')) return [await readFile(path, 'utf8')];
    const sources: string[] = [];
    for (const entry of await readdir(path, { withFileTypes: true })) {
      const child = resolve(path, entry.name);
      if (entry.isDirectory()) sources.push(...await testSources(child));
      else if (entry.isFile() && entry.name.endsWith('.rs')) sources.push(...await testSources(child));
      else if (entry.isSymbolicLink()) throw new Error(`Unsafe Rust test path: ${child}`);
    }
    return sources;
  }
  for (const module of ['process', 'artifacts', 'webhook'] as const) {
    const directory = resolve(root, `core/woml-engine/src/${module}`);
    let sources: string[];
    try {
      sources = await testSources(resolve(directory, 'tests.rs'));
    } catch (error: unknown) {
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
      try {
        sources = await testSources(resolve(directory, 'tests'));
      } catch (nested: unknown) {
        if ((nested as NodeJS.ErrnoException).code !== 'ENOENT') throw nested;
        const facade = await readFile(`${directory}.rs`, 'utf8');
        const inline = /\bmod tests \{([\s\S]*)\}\s*$/.exec(facade);
        if (inline === null) throw new Error(`Missing frozen ${module} tests.`);
        sources = [inline[1]!.replace(/^  /gm, '')];
      }
    }
    const actual = sources.flatMap(source => declarationFingerprints(
      source.replaceAll('\r\n', '\n').replace(/"(?:\.\.\/)+woml\//g, '"REPOSITORY/woml/')));
    const sort = (items: { name: string; sha256: string }[]): string[] =>
      items.map(item => `${item.name}:${item.sha256}`).sort();
    const contract = module === 'webhook' ? featureBridgeBaseline.contracts.webhook
      : capabilityArtifactBaseline.contracts[module];
    expect(sort(actual)).toEqual(sort(contract.testDeclarations));
  }
});

function orderedDeclarationHash(source: string, name: string): string {
  const matches = [...source.matchAll(new RegExp(`^(?:  )?(?:pub(?:\\([^)]*\\))? )?fn ${name}\\b`, 'gm'))];
  const match = matches.find(candidate => {
    if (!candidate[0].startsWith('  ')) return true;
    // Store and DAG adapter can expose the same method name. Compare the
    // original owning implementation, not whichever file sorts first.
    return [...source.slice(0, candidate.index).matchAll(/^impl[^\n]*/gm)].at(-1)?.[0]
      === 'impl DurableEventStore {';
  });
  if (match === undefined) throw new Error(`Missing frozen declaration ${name}.`);
  const tokens = source.slice(match.index)
    .replace(/\bpub\((?:super|in crate::durable)\)\s*/g, '')
    .match(/r#*"[\s\S]*?"#*|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])'|\/\/[^\n]*|\/\*[\s\S]*?\*\/|[^\s]/g)!
    .filter(token => !token.startsWith('//') && !token.startsWith('/*'));
  let depth = 0;
  for (let index = 0; index < tokens.length; index++) {
    if (tokens[index] === '{') depth++;
    if (tokens[index] === '}' && --depth === 0) {
      const body = tokens.slice(0, index + 1);
      return hash(body.filter((token, position) =>
        !(token === ',' && [')', ']', '}'].includes(body[position + 1]!))).join('\0'));
    }
  }
  throw new Error(`Incomplete frozen declaration ${name}.`);
}

function publicReexportNames(source: string): string[] {
  return [...source.matchAll(/^pub\s+use\s+([^;]+);/gm)].flatMap(match => {
    const tree = match[1]!;
    if (/\*|\bas\b/u.test(tree)) throw new Error('Review wildcard/aliased public re-export explicitly.');
    // Current facades use explicit path leaves and grouped leaves, not aliases.
    const compact = tree.replace(/\s/gu, '');
    const leaves = [...compact.matchAll(/(?:^|[,{])((?:\w+::)*\w+)(?=[,}]|$)/g)];
    if (leaves.length === 0) throw new Error(`Unrecognized public re-export: ${tree}`);
    return leaves.map(leaf => leaf[1]!.split('::').at(-1)!);
  });
}

test('owned implementation modules cannot leak through public module or unreviewed re-export paths', async () => {
  for (const [module, contract] of Object.entries({ ...baseline.contracts, ...executionBaseline.contracts,
    ...durableBaseline.contracts, ...capabilityArtifactBaseline.contracts, ...featureBridgeBaseline.contracts })) {
    const entry = resolve(root, 'path' in contract ? contract.path : `core/woml-engine/src/${module}.rs`);
    const source = await readRustModuleSource(entry);
    expect(source).not.toMatch(/^pub\s+mod\s/m);
    const allowed = new Set<string>([
      ...contract.publicTypes,
      ...('publicFunctions' in contract ? contract.publicFunctions : []),
      ...('publicConstants' in contract ? contract.publicConstants : []),
      ...('publicTraits' in contract ? contract.publicTraits : []),
      // RCM0 inventoried event types only. These original three public
      // validators are also pinned by the compile-time contract-path test.
      ...(module === 'event' ? ['is_approval_request_id', 'is_definition_hash', 'is_notification_delivery_id'] : []),
    ]);
    const names = publicReexportNames(source);
    expect(names.filter(name => !allowed.has(name))).toEqual([]);
  }
  expect(publicReexportNames('pub use child::{Record, nested::read};')).toEqual(['Record', 'read']);
  expect(publicReexportNames('pub use child::implementation;')).toEqual(['implementation']);
  expect(() => publicReexportNames('pub use child::*;')).toThrow('explicitly');
  expect(() => publicReexportNames('pub use child::Record as PublicAlias;')).toThrow('explicitly');
});

test('critical transaction declarations preserve ordered tokens with explicit post-refactor repairs', async () => {
  const source = (await readRustModuleSource(resolve(root, 'core/woml-engine/src/durable.rs')))
    .replaceAll('\r\n', '\n');
  for (const declaration of transactionBaseline.criticalDeclarations) {
    const repair = (compatibilityRepairs.repairs as Record<string, { originalSha256: string; sha256: string }>)[declaration.name];
    if (repair !== undefined) expect(repair.originalSha256).toBe(declaration.sha256);
    expect(orderedDeclarationHash(source, declaration.name)).toBe(repair?.sha256 ?? declaration.sha256);
  }
  expect(Object.keys(compatibilityRepairs.repairs).sort()).toEqual([
    'register_durable_wait_atomically', 'request_run_cancellation',
  ]);
  // This is a refactor-only guard, not an algorithm test or a perpetual ban on
  // reviewed behavior changes. Executable contention/recovery tests remain required.
  const name = 'recovered_state_mutation_settlement';
  const altered = source.replace('woml.state-result\\0v1\\0', 'woml.state-result\\0v2\\0');
  expect(orderedDeclarationHash(altered, name)).not.toBe(orderedDeclarationHash(source, name));
});

test('contract facades preserve the frozen public type inventories', async () => {
  for (const [module, contract] of Object.entries({ ...baseline.contracts, ...executionBaseline.contracts, ...durableBaseline.contracts, ...capabilityArtifactBaseline.contracts, ...featureBridgeBaseline.contracts })) {
    const entry = resolve(root, 'path' in contract ? contract.path : `core/woml-engine/src/${module}.rs`);
    const source = await readRustModuleSource(entry);
    const names = [...source.matchAll(/^pub (?:struct|enum|type) (\w+)/gm)].map(match => match[1]!).sort();
    expect(names).toEqual([...contract.publicTypes].sort());
    expect(new Set(names).size).toBe(names.length);
    const facade = await readFile(entry, 'utf8');
    expect(facade).not.toMatch(/pub\s+mod\s/);
    expect(facade).not.toMatch(/pub\s+use\s+[^;]*\*/);
  }
});

test('execution facades preserve their frozen public functions, constants, and traits', async () => {
  for (const [module, contract] of Object.entries({ ...executionBaseline.contracts, ...durableBaseline.contracts, ...capabilityArtifactBaseline.contracts, ...featureBridgeBaseline.contracts })) {
    const source = await readRustModuleSource(resolve(root, 'path' in contract ? contract.path : `core/woml-engine/src/${module}.rs`));
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
      else if (entry.isSymbolicLink()) throw new Error(`Unsafe fixture symlink: ${path}`);
    }
    return result.sort();
  }
  const paths = await files(resolve(root, baseline.fixtureTree.root));
  const generated = new Set(fixtureBaseline.generatedEditorFiles);
  const reviewed = paths.filter(path => !generated.has(relative(root, path).replaceAll('\\', '/')));
  expect(reviewed.map(path => relative(root, path).replaceAll('\\', '/')).sort())
    .toEqual(fixtureBaseline.files.map(file => file.path).sort());
  const expected = new Map(fixtureBaseline.files.map(file => [file.path, file.sha256]));
  // Hash canonical slash-separated relative paths so Windows and Unix agree.
  const entries = await Promise.all(reviewed.map(async path => {
    const name = relative(root, path).replaceAll('\\', '/');
    // Git may check text out as CRLF on Windows; freeze content, not autocrlf.
    const text = (await readFile(path, 'utf8')).replaceAll('\r\n', '\n');
    const expectedHash = expected.get(name);
    if (expectedHash === undefined) throw new Error(`Unreviewed source fixture: ${name}`);
    expect(hash(text), name).toBe(expectedHash);
    return `${name}\0${hash(text)}\n`;
  }));
  expect(reviewed.length).toBe(fixtureBaseline.files.length);
  expect(hash(entries.sort().join(''))).toBe(fixtureBaseline.sha256);
});
