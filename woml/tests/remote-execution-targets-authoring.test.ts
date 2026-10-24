import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import Ajv2020 from 'ajv/dist/2020';

import {
  buildWomlExecutableDefinitionPackage,
  compileWoml,
  parseWoml,
  WomlCompileError,
  WomlValidationError,
} from '../src';

const fixtureRoot = resolve(import.meta.dir, 'fixtures/remote-execution-targets');
const schemaRoot = resolve(import.meta.dir, '../../docs/schemas');
const fixture = (name: string): string =>
  readFileSync(resolve(fixtureRoot, name), 'utf8');

function workflow(step: string): string {
  return `<woml><workflow id="remote-test"><triggers><manual id="start" /></triggers><steps>${step}</steps></workflow></woml>`;
}

function diagnostic(source: string): WomlValidationError | WomlCompileError {
  try {
    compileWoml(parseWoml(source, { file: 'remote-test.woml' }));
  } catch (error) {
    if (
      error instanceof WomlValidationError ||
      error instanceof WomlCompileError
    ) {
      return error;
    }
    throw error;
  }
  throw new Error('Expected remote execution authoring to fail.');
}

describe('WOML Remote Execution Targets', () => {
  test('freezes fail-closed registry, worker, dispatch, inspection, and model schemas', () => {
    const ajv = new Ajv2020({ allErrors: true, strict: false });
    ajv.addFormat('date-time', {
      validate: (value: string) => Number.isFinite(Date.parse(value)),
    });
    for (const name of [
      'dispatch-package.v1.schema.json',
      'target-registry.v1.schema.json',
      'target-trust.v1.schema.json',
      'worker-configuration.v1.schema.json',
      'worker-protocol.v1.schema.json',
      'remote-dispatch-inspection.v1.schema.json',
      'remote-dispatch-reconciliation.v1.schema.json',
      'run-event.v20.schema.json',
      'compiled-workflow-model.v21.schema.json',
      'woml-definition-package.v16.schema.json',
    ]) {
      ajv.addSchema(JSON.parse(readFileSync(resolve(schemaRoot, name), 'utf8')));
    }

    const cases = [
      ['https://woml.dev/schemas/target-registry/v1', 'target-registry.v1.json'],
      ['https://woml.dev/schemas/target-trust/v1', 'target-trust.v1.json'],
      ['https://woml.dev/schemas/worker-configuration/v1', 'worker-configuration.v1.json'],
      ['https://woml.dev/schemas/dispatch-package/v1', 'dispatch-package.v1.json'],
      ['https://woml.dev/schemas/remote-dispatch-inspection/v1', 'inspection.v1.json'],
      ['https://woml.dev/schemas/remote-dispatch-reconciliation/v1', 'reconciliation.v1.json'],
      ['https://woml.dev/schemas/run-event/v20', 'run-event.v20.json'],
      ['https://woml.dev/schemas/compiled-workflow-model/v21', 'model.v21.reviewed.json'],
    ] as const;
    for (const [schema, name] of cases) {
      const validate = ajv.getSchema(schema)!;
      const value = JSON.parse(fixture(name));
      expect(validate(value), `${name}: ${JSON.stringify(validate.errors)}`).toBe(
        true
      );
      expect(validate({ ...value, unexpected: true })).toBe(false);
    }

    const registry = JSON.parse(fixture('target-registry.v1.json'));
    expect(
      ajv.getSchema('https://woml.dev/schemas/target-registry/v1')!({
        ...registry,
        version: 2,
      })
    ).toBe(false);
    const worker = JSON.parse(fixture('worker-configuration.v1.json'));
    expect(
      ajv.getSchema('https://woml.dev/schemas/worker-configuration/v1')!({
        ...worker,
        version: 2,
      })
    ).toBe(false);
    const model = JSON.parse(fixture('model.v21.reviewed.json'));
    expect(
      ajv.getSchema('https://woml.dev/schemas/compiled-workflow-model/v21')!({
        ...model,
        schemaVersion: 22,
      })
    ).toBe(false);

    const validateMessage = ajv.getSchema(
      'https://woml.dev/schemas/worker-protocol/v1'
    )!;
    for (const message of JSON.parse(fixture('worker-protocol.v1.json'))) {
      expect(
        validateMessage(message),
        `${message.messageType}: ${JSON.stringify(validateMessage.errors)}`
      ).toBe(true);
      expect(validateMessage({ ...message, unexpected: true })).toBe(false);
      expect(validateMessage({ ...message, protocolVersion: 2 })).toBe(false);
    }
  });

  test('deep-equals the reviewed Model v21 fixture and preserves correction placement', () => {
    const source = fixture('model.reviewed.woml');
    const reviewed = JSON.stringify(
      JSON.parse(fixture('model.v21.reviewed.json'))
    );
    const compiled = compileWoml(
      parseWoml(source, {
        file: 'model.reviewed.woml',
      })
    );
    expect(JSON.stringify(compiled)).toBe(reviewed);
    expect(
      JSON.stringify(
        compileWoml(
          parseWoml(
            source.replace(/\r\n?/g, '\n').replaceAll('\n', '\r\n'),
            {
              file: 'model.reviewed.woml',
            }
          )
        )
      )
    ).toBe(reviewed);
    expect(compiled.schemaVersion).toBe(21);
    if (compiled.schemaVersion !== 21) throw new Error('Expected Model v21.');

    const deployed = compiled.graph.nodes.find(node => node.id === 'applyRelease')!;
    expect(deployed.execution).toEqual({
      profileVersion: 1,
      target: 'production-private',
      requiredCapabilities: ['runtime.script'],
    });
    const saga = compiled.graph.sagas[0]!;
    expect(saga.compensations[0]!.node.execution).toEqual(deployed.execution);
    expect(saga.cleanup[0]!.node.execution).toEqual({
      profileVersion: 1,
      target: 'maintenance-runner',
      requiredCapabilities: ['runtime.script'],
    });
  });

  test('keeps workflows without explicit placement on their previous model', () => {
    const source = workflow(
      '<step id="local"><script>return { local: true };</script></step>'
    );
    const first = compileWoml(parseWoml(source));
    const second = compileWoml(parseWoml(source));
    expect(first.schemaVersion).toBe(1);
    expect(JSON.stringify(first)).toBe(JSON.stringify(second));
    expect(first.graph.nodes[0]!.execution).toBeUndefined();
  });

  test('treats local as an explicit target rather than an implicit fallback', () => {
    const compiled = compileWoml(
      parseWoml(
        workflow(
          '<step id="placed" target="local"><script>return true;</script></step>'
        )
      )
    );
    expect(compiled.schemaVersion).toBe(21);
    expect(compiled.graph.nodes[0]!.execution).toEqual({
      profileVersion: 1,
      target: 'local',
      requiredCapabilities: ['runtime.script'],
    });
  });

  test('binds reviewed built-in service requirements into placement identity', () => {
    const compiled = compileWoml(
      parseWoml(
        workflow(
          '<step id="checks" target="ci-runner"><script>return services.process.run({ executable: "bun", args: ["test"] });</script></step>'
        )
      )
    );
    expect(compiled.schemaVersion).toBe(21);
    expect(compiled.graph.nodes[0]!.execution).toEqual({
      profileVersion: 1,
      target: 'ci-runner',
      requiredCapabilities: ['runtime.script', 'services.process'],
    });
  });

  test('preserves placement through retry, loop, branch, parallel, and fork lowering', () => {
    const cases = [
      [
        resolve(import.meta.dir, 'fixtures/for-each/model.reviewed.woml'),
        'id="normalize"',
        'normalize',
      ],
      [
        resolve(import.meta.dir, 'fixtures/retry-composition.woml'),
        'id="leftCheck"',
        'leftCheck',
      ],
      [
        resolve(import.meta.dir, 'fixtures/fork-branch/join-all.woml'),
        'id="publishInstagram"',
        'publishInstagram',
      ],
    ] as const;

    for (const [path, marker, stepId] of cases) {
      const source = readFileSync(path, 'utf8').replace(
        marker,
        `${marker} target="private-runner"`
      );
      const compiled = compileWoml(parseWoml(source, { file: path }));
      expect(compiled.schemaVersion).toBe(21);
      if (compiled.schemaVersion !== 21) throw new Error('Expected Model v21.');
      const nodes = [
        ...compiled.graph.nodes,
        ...compiled.graph.forEach.flatMap(loop => loop.body.nodes),
      ];
      expect(nodes.find(node => node.id === stepId)?.execution).toEqual({
        profileVersion: 1,
        target: 'private-runner',
        requiredCapabilities: ['runtime.script'],
      });
    }
  });

  test('rejects dynamic, malformed, and oversized targets at the attribute value', () => {
    const invalidTargets = [
      '{{context.payload.target}}',
      'Production',
      'local/path',
      '',
      `a${'b'.repeat(128)}`,
    ];
    for (const target of invalidTargets) {
      const error = diagnostic(
        workflow(
          `<step id="placed" target="${target}"><script>return true;</script></step>`
        )
      );
      expect(error.diagnostic.code).toBe('WOML_TARGET_INVALID');
      expect(error.diagnostic.file).toBe('remote-test.woml');
      expect(error.diagnostic.location.start.line).toBeGreaterThan(0);
    }
  });

  test('rejects target placement on structural elements', () => {
    const error = diagnostic(
      workflow(
        '<parallel id="group" target="private-runner"><step id="placed"><script>return true;</script></step></parallel>'
      )
    );
    expect(error.diagnostic.code).toBe('WOML_UNKNOWN_ATTRIBUTE');
  });

  test('packages module-backed placement deterministically as Definition Package v16', async () => {
    const path = resolve(fixtureRoot, 'model-module.reviewed.woml');
    const document = parseWoml(readFileSync(path, 'utf8'), { file: path });
    const first = await buildWomlExecutableDefinitionPackage(document, {
      sourcePath: path,
      projectRoot: fixtureRoot,
    });
    const second = await buildWomlExecutableDefinitionPackage(document, {
      sourcePath: path,
      projectRoot: fixtureRoot,
    });

    expect(first.schemaVersion).toBe(16);
    expect(first.profile).toBe('woml.definition-package/v16');
    expect(first.runtimeReady).toBe(false);
    expect(first.workflow.model.schemaVersion).toBe(21);
    expect(first.rootHash).toBe(second.rootHash);

    const ajv = new Ajv2020({ allErrors: true, strict: false });
    for (const name of [
      'compiled-workflow-model.v21.schema.json',
      'woml-definition-package.v16.schema.json',
    ]) {
      ajv.addSchema(JSON.parse(readFileSync(resolve(schemaRoot, name), 'utf8')));
    }
    const validate = ajv.getSchema(
      'https://woml.dev/schemas/woml-definition-package.v16.schema.json'
    )!;
    expect(validate(first), JSON.stringify(validate.errors)).toBe(true);
    expect(validate({ ...first, unexpected: true })).toBe(false);
  });
});
