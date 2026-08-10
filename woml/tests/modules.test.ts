import { describe, expect, test } from 'bun:test';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import Ajv2020 from 'ajv/dist/2020';

import {
  buildWomlDefinitionPackage,
  canonicalizeWomlDefinitionPackage,
  compileWoml,
  parseWoml,
  validateWoml,
  WomlCompileError,
  WomlValidationError,
} from '../src';

const fixtureRoot = resolve(import.meta.dir, 'fixtures/modules');
const workflowPath = resolve(fixtureRoot, 'customer-import.woml');

function sourceDocument(path = workflowPath) {
  return parseWoml(readFileSync(path, 'utf8'), { file: path });
}

function moduleDocument(name: string, from: string, file = workflowPath) {
  return parseWoml(
    `<woml>
  <imports><module name="${name}" from="${from}" /></imports>
  <workflow id="module-test">
    <triggers><manual id="start" /></triggers>
    <steps><step id="done"><script>return true;</script></step></steps>
  </workflow>
</woml>`,
    { file }
  );
}

function validationCode(source: string): string {
  try {
    validateWoml(parseWoml(source, { file: 'invalid.woml' }));
  } catch (error) {
    expect(error).toBeInstanceOf(WomlValidationError);
    return (error as WomlValidationError).diagnostic.code;
  }
  throw new Error('Expected WOML validation to fail.');
}

describe('MS0 and MS1 Module System', () => {
  test('deep-equals the reviewed immutable package and validates its schema', () => {
    const actual = buildWomlDefinitionPackage(sourceDocument(), {
      sourcePath: workflowPath,
      projectRoot: fixtureRoot,
    });
    const expected = JSON.parse(
      readFileSync(
        resolve(fixtureRoot, 'customer-import.package.v1.json'),
        'utf8'
      )
    );
    const schema = JSON.parse(
      readFileSync(
        resolve(import.meta.dir, '../../docs/schemas/woml-definition-package.v1.schema.json'),
        'utf8'
      )
    );
    const validate = new Ajv2020({ strict: false }).compile(schema);

    expect(actual).toEqual(expected);
    expect(validate(actual)).toBe(true);
    expect(canonicalizeWomlDefinitionPackage(actual)).toBe(
      canonicalizeWomlDefinitionPackage(expected)
    );
  });

  test('requires the canonical wrapper and ordered document children', () => {
    expect(
      validationCode(`<workflow id="legacy">
  <triggers><manual id="start" /></triggers>
  <steps><step id="done"><script>return true;</script></step></steps>
</workflow>`)
    ).toBe('WOML_EXPECTED_DOCUMENT_ROOT');
    expect(
      validationCode(`<woml>
  <workflow id="wrong-order">
    <triggers><manual id="start" /></triggers>
    <steps><step id="done"><script>return true;</script></step></steps>
  </workflow>
  <imports><module name="tools" from="./tools.ts" /></imports>
</woml>`)
    ).toBe('WOML_DOCUMENT_STRUCTURE_INVALID');
    expect(validationCode('<woml><imports /></woml>')).toBe(
      'WOML_DOCUMENT_STRUCTURE_INVALID'
    );
  });

  test('freezes module aliases, paths, emptiness, and the code-only boundary', () => {
    const workflow = (declaration: string) => `<woml>
  <imports>${declaration}</imports>
  <workflow id="shape-test">
    <triggers><manual id="start" /></triggers>
    <steps><step id="done"><script>return true;</script></step></steps>
  </workflow>
</woml>`;
    expect(
      validationCode(workflow('<module name="http" from="./http.ts" />'))
    ).toBe('WOML_MODULE_ALIAS_RESERVED');
    expect(
      validationCode(workflow('<module name="Bad-name" from="./tools.ts" />'))
    ).toBe('WOML_MODULE_ALIAS_INVALID');
    expect(
      validationCode(workflow('<module name="tools" from="https://example.test/tools.ts" />'))
    ).toBe('WOML_MODULE_PATH_INVALID');
    expect(
      validationCode(workflow('<module name="child" from="./child.woml" />'))
    ).toBe('WOML_MODULE_WORKFLOW_UNSUPPORTED');
    expect(
      validationCode(workflow('<module name="tools" from="./tools.ts">bad</module>'))
    ).toBe('WOML_INVALID_STRUCTURE');
  });

  test('resolves a deterministic transitive graph without executing module code', () => {
    const first = buildWomlDefinitionPackage(sourceDocument(), {
      sourcePath: workflowPath,
      projectRoot: fixtureRoot,
    });
    const second = buildWomlDefinitionPackage(sourceDocument(), {
      sourcePath: workflowPath,
      projectRoot: fixtureRoot,
    });
    expect(second).toEqual(first);
    expect(first.sources.map(source => source.path)).toEqual([
      'customer-import.woml',
      'spreadsheet.ts',
      'values.ts',
    ]);
    expect(first.modules[0]?.exports).toEqual(['read', 'removeEmptyRows']);
    expect(first.executable).toBe(false);
    expect(() => compileWoml(sourceDocument())).toThrow(WomlCompileError);
    try {
      compileWoml(sourceDocument());
    } catch (error) {
      expect((error as WomlCompileError).diagnostic.code).toBe(
        'WOML_MODULE_EXECUTION_UNAVAILABLE'
      );
    }
  });

  test('changes the root hash when exact source bytes change', () => {
    const directory = mkdtempSync(join(tmpdir(), 'woml-ms1-hash-'));
    try {
      const womlPath = join(directory, 'workflow.woml');
      const modulePath = join(directory, 'tools.ts');
      const womlSource = `<woml>
  <imports><module name="tools" from="./tools.ts" /></imports>
  <workflow id="hash-test"><triggers><manual id="start" /></triggers><steps><step id="done"><script>return true;</script></step></steps></workflow>
</woml>`;
      writeFileSync(womlPath, womlSource);
      writeFileSync(modulePath, 'export function value() { return 1; }\n');
      const first = buildWomlDefinitionPackage(
        parseWoml(womlSource, { file: womlPath }),
        { sourcePath: womlPath, projectRoot: directory }
      );
      writeFileSync(modulePath, 'export function value() { return 2; }\n');
      const second = buildWomlDefinitionPackage(
        parseWoml(womlSource, { file: womlPath }),
        { sourcePath: womlPath, projectRoot: directory }
      );
      expect(second.rootHash).not.toBe(first.rootHash);
      expect(second.sources[1]?.digest).not.toBe(first.sources[1]?.digest);
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });

  test('rejects unsupported exports and cyclic graphs with stable diagnostics', () => {
    try {
      buildWomlDefinitionPackage(
        moduleDocument('broken', './invalid-default.ts'),
        { sourcePath: workflowPath, projectRoot: fixtureRoot }
      );
      throw new Error('Expected a default-export failure.');
    } catch (error) {
      expect(error).toBeInstanceOf(WomlCompileError);
      expect((error as WomlCompileError).diagnostic.code).toBe(
        'WOML_MODULE_DEFAULT_EXPORT_UNSUPPORTED'
      );
      expect((error as WomlCompileError).diagnostic.file).toBe(
        'invalid-default.ts'
      );
    }

    const unsupportedSources = [
      ['./invalid-dynamic.ts', 'WOML_MODULE_DYNAMIC_IMPORT_UNSUPPORTED'],
      ['./invalid-commonjs.js', 'WOML_MODULE_COMMONJS_UNSUPPORTED'],
      ['./invalid-generator.js', 'WOML_MODULE_GENERATOR_EXPORT_UNSUPPORTED'],
    ] as const;
    for (const [from, code] of unsupportedSources) {
      try {
        buildWomlDefinitionPackage(moduleDocument('broken', from), {
          sourcePath: workflowPath,
          projectRoot: fixtureRoot,
        });
        throw new Error(`Expected ${code}.`);
      } catch (error) {
        expect(error).toBeInstanceOf(WomlCompileError);
        expect((error as WomlCompileError).diagnostic.code).toBe(code);
      }
    }

    try {
      buildWomlDefinitionPackage(
        moduleDocument('cycle', './invalid-cycle-a.ts'),
        { sourcePath: workflowPath, projectRoot: fixtureRoot }
      );
      throw new Error('Expected a cycle failure.');
    } catch (error) {
      expect(error).toBeInstanceOf(WomlCompileError);
      expect((error as WomlCompileError).diagnostic.code).toBe(
        'WOML_MODULE_GRAPH_CYCLE'
      );
    }
  });

  test('rejects missing files and project-boundary escapes before packaging', () => {
    for (const [from, code] of [
      ['./missing.ts', 'WOML_MODULE_FILE_NOT_FOUND'],
      ['../../outside.ts', 'WOML_MODULE_PATH_ESCAPE'],
    ] as const) {
      try {
        buildWomlDefinitionPackage(moduleDocument('unsafe', from), {
          sourcePath: workflowPath,
          projectRoot: fixtureRoot,
        });
        throw new Error(`Expected ${code}.`);
      } catch (error) {
        expect(error).toBeInstanceOf(WomlCompileError);
        expect((error as WomlCompileError).diagnostic.code).toBe(code);
      }
    }
  });
});
