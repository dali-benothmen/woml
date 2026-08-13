import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import Ajv2020 from 'ajv/dist/2020';

import {
  buildWomlReusableDefinitionPackage,
  parseWoml,
  resolveWomlReusableDefinitionGraph,
} from '../src';

const repositoryRoot = resolve(import.meta.dir, '../..');
const schemaRoot = resolve(repositoryRoot, 'docs/schemas');
const fixtureRoot = resolve(import.meta.dir, 'fixtures/reusable-definitions');

function json(path: string): any {
  return JSON.parse(readFileSync(path, 'utf8'));
}

function fixture(name: string): any {
  return json(resolve(fixtureRoot, name));
}

function validators(): Ajv2020 {
  const ajv = new Ajv2020({ strict: false, allErrors: true });
  ajv.addFormat('date-time', /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z$/);
  ajv.addFormat('uri', /^https?:\/\/[^\s]+$/);
  const names = [
    ...Array.from({ length: 14 }, (_, index) =>
      `compiled-workflow-model.v${index + 1}.schema.json`
    ),
    ...Array.from({ length: 13 }, (_, index) =>
      `run-event.v${index + 1}.schema.json`
    ),
    ...Array.from({ length: 9 }, (_, index) =>
      `woml-definition-package.v${index + 1}.schema.json`
    ),
    'runtime-policy.v1.schema.json',
    'woml-template.v1.schema.json',
    'run-inspection.v2.schema.json',
    'run-inspection.v3.schema.json',
    'run-inspection.v4.schema.json',
    'run-inspection.v5.schema.json',
    'reusable-script-binding.v3.schema.json',
    'custom-notification-provider.v1.schema.json',
  ];
  for (const name of names) {
    ajv.addSchema(json(resolve(schemaRoot, name)));
  }
  return ajv;
}

function expectValid(validate: any, value: unknown): void {
  expect(validate(value), JSON.stringify(validate.errors, null, 2)).toBe(true);
}

describe('frozen reusable definition contracts', () => {
  test('validates a compiler-produced reusable-step package', async () => {
    const path = resolve(fixtureRoot, 'custom-step-workflow.woml');
    const document = parseWoml(readFileSync(path, 'utf8'), { file: path });
    const graph = resolveWomlReusableDefinitionGraph(document, {
      sourcePath: path,
      projectRoot: fixtureRoot,
    });
    const definitionPackage = await buildWomlReusableDefinitionPackage(
      document,
      graph,
      { sourcePath: path, projectRoot: fixtureRoot }
    );
    const ajv = validators();
    expectValid(
      ajv.getSchema('https://cronflow.dev/schemas/compiled-workflow-model/v14'),
      definitionPackage.workflow.model
    );
    expectValid(
      ajv.getSchema('https://woml.dev/schemas/woml-definition-package.v9.schema.json'),
      definitionPackage
    );
  });

  test('validates Model v14 and Definition Package v9 reviewed artifacts', () => {
    const ajv = validators();
    const model = fixture('model-v14.reviewed.json');
    expectValid(
      ajv.getSchema('https://cronflow.dev/schemas/compiled-workflow-model/v14'),
      model
    );

    const hash = `sha256:${'0'.repeat(64)}`;
    const packageFixture = {
      schemaVersion: 9,
      profile: 'woml.definition-package/v9',
      executable: true,
      runtimeReady: false,
      workflow: {
        id: model.workflowId,
        source: 'workflow.woml',
        modelDigest: hash,
        model,
      },
      definitions: [
        {
          alias: 'calculate-discount',
          kind: 'reusable-step',
          source: 'calculate-discount.woml',
          digest: `sha256:${'1'.repeat(64)}`,
          dependencies: ['pricing.ts'],
          props: [
            { name: 'price', bindingName: 'price', required: true, secret: false },
            { name: 'percentage', bindingName: 'percentage', required: true, secret: false },
          ],
        },
        {
          alias: 'telegram',
          kind: 'notification-provider',
          source: 'telegram.woml',
          digest: `sha256:${'2'.repeat(64)}`,
          dependencies: [],
          props: [
            { name: 'bot-token', bindingName: 'botToken', required: true, secret: true },
            { name: 'chat-id', bindingName: 'chatId', required: true, secret: false },
          ],
        },
      ],
      modules: [],
      sources: [
        { path: 'workflow.woml', mediaType: 'application/woml+xml', digest: hash, dependencies: ['calculate-discount.woml', 'telegram.woml'] },
        { path: 'calculate-discount.woml', mediaType: 'application/woml+xml', digest: `sha256:${'1'.repeat(64)}`, dependencies: ['pricing.ts'] },
        { path: 'telegram.woml', mediaType: 'application/woml+xml', digest: `sha256:${'2'.repeat(64)}`, dependencies: [] },
      ],
      artifacts: [
        { path: 'model-v14.json', kind: 'workflow-model', mediaType: 'application/json', digest: hash, content: '{}' },
        { path: 'calculate-discount.js', kind: 'module-bundle', mediaType: 'text/javascript', digest: `sha256:${'3'.repeat(64)}`, content: 'export default async function () {}' },
      ],
      compiler: {
        name: 'woml', version: '0.1.0', resolverProfile: 'woml.module-resolver/v1',
        bundler: { name: 'bun', version: '1.3.14', target: 'bun', format: 'esm', sourceMap: 'external' },
      },
      permissions: { secrets: ['TELEGRAM_BOT_TOKEN'], networkOrigins: ['https://api.telegram.org'] },
      rootHash: hash,
    };
    expectValid(
      ajv.getSchema('https://woml.dev/schemas/woml-definition-package.v9.schema.json'),
      packageFixture
    );
  });

  test('validates Script Bindings v3, Event v13, and redacted Inspection v5', () => {
    const ajv = validators();
    const contracts = fixture('contracts.v1.json');
    expectValid(
      ajv.getSchema('https://woml.dev/schemas/reusable-script-binding/v3'),
      contracts.scriptBinding
    );
    const validateEvent = ajv.getSchema('https://cronflow.dev/schemas/run-event/v13');
    for (const event of fixture('event-history.v13.json')) expectValid(validateEvent, event);
    expectValid(
      ajv.getSchema('https://woml.dev/schemas/run-inspection/v5'),
      contracts.inspection
    );
    expect(contracts.storeDecision).toEqual(expect.objectContaining({
      storeVersion: 14,
      migrationRequired: false,
    }));
    expect(JSON.stringify(contracts.inspection)).not.toMatch(
      /botToken|TELEGRAM_BOT_TOKEN|approvalUrl|rejectUrl|"props"/
    );
  });

  test('validates asynchronous custom-provider messages and UTF-8 byte framing', () => {
    const ajv = validators();
    const messages = fixture('provider-protocol.v1.json');
    const validate = ajv.getSchema(
      'https://woml.dev/schemas/custom-notification-provider/v1'
    );
    for (const message of Object.values(messages)) expectValid(validate, message);

    const encoded = JSON.stringify(messages.execute);
    expect(encoded).toContain('\\r\\n');
    expect(Buffer.byteLength(encoded, 'utf8')).toBeGreaterThan(encoded.length);
    expect(messages.completed.invocationId).toBe(messages.execute.invocationId);
  });

  test('keeps historical model, event, package, and inspection schemas loadable', () => {
    const ajv = validators();
    expect(ajv.getSchema('https://cronflow.dev/schemas/compiled-workflow-model/v13')).toBeDefined();
    expect(ajv.getSchema('https://cronflow.dev/schemas/run-event/v12')).toBeDefined();
    expect(ajv.getSchema('https://woml.dev/schemas/woml-definition-package.v8.schema.json')).toBeDefined();
    expect(ajv.getSchema('https://woml.dev/schemas/run-inspection/v4')).toBeDefined();
  });

  test('freezes lifecycle, recovery, shared approval, and result semantics', () => {
    const semantics = fixture('semantics.v1.json');
    expect(semantics.lifecycleOrder.success).toEqual([
      'operation-success-committed',
      'on-success',
      'on-complete',
      'downstream-released',
    ]);
    expect(semantics.lifecycleOrder.permanentFailure).toEqual([
      'operation-failure-committed',
      'on-error',
      'on-complete',
      'workflow-failure-settled',
    ]);
    expect(semantics.recovery).toMatchObject({
      definitionSource: 'pinned-package-v9',
      ambiguousEffect: 'fail-closed',
    });
    expect(semantics.approval).toMatchObject({
      capabilities: 'separate-per-delivery',
      decisionAuthority: 'one-shared-approval',
      settlement: 'first-valid-decision-wins',
    });
    expect(semantics.results).toEqual({
      customStep: 'context.steps.invocationId',
      customProvider: 'none',
      controlSwitch: 'none',
      resultSwitch: 'context.steps.switchId',
    });
  });
});
