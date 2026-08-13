import { describe, expect, test } from 'bun:test';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import Ajv2020 from 'ajv/dist/2020';

import {
  buildWomlDefinitionPackage,
  buildWomlExecutableDefinitionPackage,
  buildWomlRuntimeDefinitionPackage,
  canonicalizeWomlDefinitionPackage,
  compileWoml,
  inspectWomlModuleUsage,
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

  test('freezes module aliases, paths, emptiness, and extension-specific imports', () => {
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
    expect(() =>
      validateWoml(
        parseWoml(
          workflow('<module name="child-step" from="./child.woml" />'),
          { file: 'reusable-import.woml' }
        )
      )
    ).not.toThrow();
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

describe('essential MS6 module diagnostics', () => {
  test('reports referenced and unused module aliases without rejecting unused code', () => {
    const inspection = inspectWomlModuleUsage(sourceDocument());
    expect(inspection).toEqual({
      referencedServices: ['spreadsheet'],
      referencedModules: ['spreadsheet'],
      unusedModules: [],
    });

    const unused = inspectWomlModuleUsage(
      moduleDocument('spreadsheet', './spreadsheet.ts')
    );
    expect(unused.unusedModules).toEqual(['spreadsheet']);
  });

  test('rejects a misspelled service alias at its script location', () => {
    const source = `<woml>
  <workflow id="bad-service">
    <triggers><manual id="start" /></triggers>
    <steps><step id="call"><script>
      return services.spredsheet.read([]);
    </script></step></steps>
  </workflow>
</woml>`;
    try {
      compileWoml(parseWoml(source, { file: 'bad-service.woml' }));
      throw new Error('Expected unknown service alias rejection.');
    } catch (error) {
      expect(error).toBeInstanceOf(WomlCompileError);
      const diagnostic = (error as WomlCompileError).diagnostic;
      expect(diagnostic.code).toBe('WOML_MODULE_SERVICE_UNKNOWN');
      expect(diagnostic.message).toContain('spredsheet');
      expect(diagnostic.location.start.line).toBe(5);
    }
  });
});

describe('MS2 deterministic module compilation', () => {
  test('pins Model v9, bundle, source map, declarations, and package identity', async () => {
    const actual = await buildWomlExecutableDefinitionPackage(sourceDocument(), {
      sourcePath: workflowPath,
      projectRoot: fixtureRoot,
    });
    const expectedModel = JSON.parse(
      readFileSync(resolve(fixtureRoot, 'customer-import.compiled.v9.json'), 'utf8')
    );
    const expectedIdentity = JSON.parse(
      readFileSync(
        resolve(fixtureRoot, 'customer-import.package.v2.identity.json'),
        'utf8'
      )
    );
    const artifact = (path: string) => {
      const found = actual.artifacts.find(item => item.path === path);
      if (found === undefined) throw new Error(`Missing artifact ${path}.`);
      return found;
    };

    expect(actual.workflow.model).toEqual(expectedModel);
    expect(artifact('modules/spreadsheet.mjs').content).toBe(
      readFileSync(resolve(fixtureRoot, 'spreadsheet.bundle.v1.mjs'), 'utf8')
    );
    expect(artifact('modules/spreadsheet.mjs.map').content).toBe(
      readFileSync(
        resolve(fixtureRoot, 'spreadsheet.bundle.v1.mjs.map'),
        'utf8'
      ).trimEnd()
    );
    expect(artifact('types/services.generated.d.ts').content).toBe(
      readFileSync(resolve(fixtureRoot, 'services.generated.v1.d.ts'), 'utf8')
    );

    const identity = {
      schemaVersion: actual.schemaVersion,
      profile: actual.profile,
      rootHash: actual.rootHash,
      workflowModelDigest: actual.workflow.modelDigest,
      modules: actual.modules.map(module => ({
        name: module.name,
        bundle: module.bundle,
        sourceMap: module.sourceMap,
      })),
      artifacts: actual.artifacts.map(item => ({
        path: item.path,
        digest: item.digest,
      })),
      bundler: actual.compiler.bundler,
    };
    expect(identity).toEqual(expectedIdentity);
    expect(actual.executable).toBe(true);
    expect(actual.runtimeReady).toBe(false);
  });

  test('validates Definition Package v2 and Model v9 against frozen schemas', async () => {
    const actual = await buildWomlExecutableDefinitionPackage(sourceDocument(), {
      sourcePath: workflowPath,
      projectRoot: fixtureRoot,
    });
    const schemaPaths = [
      'compiled-workflow-model.v1.schema.json',
      'compiled-workflow-model.v2.schema.json',
      'compiled-workflow-model.v3.schema.json',
      'compiled-workflow-model.v4.schema.json',
      'compiled-workflow-model.v5.schema.json',
      'compiled-workflow-model.v6.schema.json',
      'compiled-workflow-model.v7.schema.json',
      'compiled-workflow-model.v8.schema.json',
      'compiled-workflow-model.v9.schema.json',
      'woml-definition-package.v1.schema.json',
      'woml-definition-package.v2.schema.json',
    ];
    const ajv = new Ajv2020({ strict: false });
    for (const schemaPath of schemaPaths) {
      ajv.addSchema(
        JSON.parse(
          readFileSync(resolve(import.meta.dir, '../../docs/schemas', schemaPath), 'utf8')
        )
      );
    }
    const validatePackage = ajv.getSchema(
      'https://woml.dev/schemas/woml-definition-package.v2.schema.json'
    );
    const validateModel = ajv.getSchema(
      'https://cronflow.dev/schemas/compiled-workflow-model/v9'
    );
    expect(validatePackage?.(actual), JSON.stringify(validatePackage?.errors)).toBe(true);
    expect(validateModel?.(actual.workflow.model), JSON.stringify(validateModel?.errors)).toBe(
      true
    );
  });

  test('reproduces byte-identical artifacts across clean build directories', async () => {
    const first = await buildWomlExecutableDefinitionPackage(sourceDocument(), {
      sourcePath: workflowPath,
      projectRoot: fixtureRoot,
    });
    const second = await buildWomlExecutableDefinitionPackage(sourceDocument(), {
      sourcePath: workflowPath,
      projectRoot: fixtureRoot,
    });
    expect(second).toEqual(first);
    expect(JSON.stringify(first)).not.toContain(fixtureRoot);
    expect(JSON.stringify(first)).not.toContain(tmpdir());
  });

  test('preserves package identity when the same project moves directories', async () => {
    const temporary = mkdtempSync(join(tmpdir(), 'woml-ms2-portable-'));
    const createProject = (name: string) => {
      const root = join(temporary, name);
      mkdirSync(root);
      for (const file of ['customer-import.woml', 'spreadsheet.ts', 'values.ts']) {
        writeFileSync(join(root, file), readFileSync(resolve(fixtureRoot, file)));
      }
      const sourcePath = join(root, 'customer-import.woml');
      return { root, sourcePath };
    };
    try {
      const left = createProject('left');
      const right = createProject('right');
      const build = async ({ root, sourcePath }: ReturnType<typeof createProject>) =>
        await buildWomlExecutableDefinitionPackage(sourceDocument(sourcePath), {
          sourcePath,
          projectRoot: root,
        });
      expect(await build(right)).toEqual(await build(left));
    } finally {
      rmSync(temporary, { recursive: true, force: true });
    }
  });
});

describe('MS3 runtime definition package', () => {
  test('promotes unchanged compilation artifacts into a deterministic runtime-ready package', async () => {
    const actual = await buildWomlRuntimeDefinitionPackage(sourceDocument(), {
      sourcePath: workflowPath,
      projectRoot: fixtureRoot,
    });
    const repeated = await buildWomlRuntimeDefinitionPackage(sourceDocument(), {
      sourcePath: workflowPath,
      projectRoot: fixtureRoot,
    });
    const schemaPaths = [
      'compiled-workflow-model.v1.schema.json',
      'compiled-workflow-model.v2.schema.json',
      'compiled-workflow-model.v3.schema.json',
      'compiled-workflow-model.v4.schema.json',
      'compiled-workflow-model.v5.schema.json',
      'compiled-workflow-model.v6.schema.json',
      'compiled-workflow-model.v7.schema.json',
      'compiled-workflow-model.v8.schema.json',
      'compiled-workflow-model.v9.schema.json',
      'woml-definition-package.v1.schema.json',
      'woml-definition-package.v3.schema.json',
    ];
    const ajv = new Ajv2020({ strict: false });
    for (const schemaPath of schemaPaths) {
      ajv.addSchema(
        JSON.parse(
          readFileSync(
            resolve(import.meta.dir, '../../docs/schemas', schemaPath),
            'utf8'
          )
        )
      );
    }
    const validate = ajv.getSchema(
      'https://woml.dev/schemas/woml-definition-package.v3.schema.json'
    );
    expect(validate?.(actual), JSON.stringify(validate?.errors)).toBe(true);
    expect(repeated).toEqual(actual);
    expect(actual).toMatchObject({
      schemaVersion: 3,
      profile: 'woml.definition-package/v3',
      executable: true,
      runtimeReady: true,
    });
    expect(actual.compilationRootHash).not.toBe(actual.rootHash);
    expect({
      schemaVersion: actual.schemaVersion,
      profile: actual.profile,
      runtimeReady: actual.runtimeReady,
      compilationRootHash: actual.compilationRootHash,
      rootHash: actual.rootHash,
      workflowModelDigest: actual.workflow.modelDigest,
      modules: actual.modules.map(module => ({
        name: module.name,
        bundle: module.bundle,
        sourceMap: module.sourceMap,
      })),
    }).toEqual(
      JSON.parse(
        readFileSync(
          resolve(fixtureRoot, 'customer-import.package.v3.identity.json'),
          'utf8'
        )
      )
    );
  });
});
