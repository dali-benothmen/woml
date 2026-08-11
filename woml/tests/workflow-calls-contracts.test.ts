import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { readdir } from 'node:fs/promises';
import { join, resolve } from 'node:path';

import Ajv2020 from 'ajv/dist/2020';

import {
  analyzeWomlScript,
  buildWomlExecutableDefinitionPackage,
  buildWomlRuntimeDefinitionPackage,
  compileWoml,
  generateWomlEditorDeclarations,
  parseWoml,
  validateWoml,
  WOML_WORKFLOW_CALL_DEFINITION_PACKAGE_PROFILE,
  WOML_WORKFLOW_CALL_RUNTIME_DEFINITION_PACKAGE_PROFILE,
  WomlCompileError,
  WomlValidationError,
} from '../src';

const repositoryRoot = resolve(import.meta.dir, '../..');
const schemaDirectory = resolve(repositoryRoot, 'docs/schemas');
const fixtureDirectory = resolve(import.meta.dir, 'fixtures/workflow-calls');
const contractDirectory = resolve(
  import.meta.dir,
  'fixtures/workflow-call-contracts'
);

async function validators() {
  const ajv = new Ajv2020({
    allErrors: true,
    strict: false,
    validateFormats: false,
  });
  for (const name of [
    'attempt-failure.v1.schema.json',
    'attempt-failure.v2.schema.json',
    'attempt-failure.v3.schema.json',
    'capability-call.v1.schema.json',
    ...Array.from(
      { length: 10 },
      (_, index) => `compiled-workflow-model.v${index + 1}.schema.json`
    ),
    ...Array.from(
      { length: 5 },
      (_, index) => `woml-definition-package.v${index + 1}.schema.json`
    ),
    ...Array.from(
      { length: 9 },
      (_, index) => `run-event.v${index + 1}.schema.json`
    ),
    'workflow-call.v1.schema.json',
    'workflow-call-index.v1.schema.json',
    'workflow-call-progress.v1.schema.json',
    'workflow-call-routing.v1.schema.json',
  ]) {
    ajv.addSchema(await Bun.file(join(schemaDirectory, name)).json());
  }
  return {
    model: ajv.getSchema(
      'https://cronflow.dev/schemas/compiled-workflow-model/v10'
    )!,
    packageV4: ajv.getSchema(
      'https://woml.dev/schemas/woml-definition-package.v4.schema.json'
    )!,
    packageV5: ajv.getSchema(
      'https://woml.dev/schemas/woml-definition-package.v5.schema.json'
    )!,
    call: ajv.getSchema('https://woml.dev/schemas/workflow-call/v1')!,
    index: ajv.getSchema(
      'https://woml.dev/schemas/workflow-call-index/v1'
    )!,
    routing: ajv.getSchema(
      'https://woml.dev/schemas/workflow-call-routing/v1'
    )!,
    progress: ajv.getSchema(
      'https://woml.dev/schemas/workflow-call-progress.v1.schema.json'
    )!,
    event: ajv.getSchema('https://cronflow.dev/schemas/run-event/v9')!,
  };
}

const schemas = await validators();

function document(name: string) {
  const path = resolve(fixtureDirectory, `${name}.woml`);
  return parseWoml(readFileSync(path, 'utf8'), { file: path });
}

function compile(source: string) {
  return compileWoml(parseWoml(source, { file: 'workflow.woml' }));
}

function scriptWorkflow(source: string) {
  return `<woml>
  <workflow id="caller">
    <triggers><manual id="start" /></triggers>
    <steps><step id="call"><script>${source}</script></step></steps>
  </workflow>
</woml>`;
}

type WorkflowCallDefinitionPackage =
  | Awaited<ReturnType<typeof buildWomlExecutableDefinitionPackage>>
  | Awaited<ReturnType<typeof buildWomlRuntimeDefinitionPackage>>;

function packageIdentity(definitionPackage: WorkflowCallDefinitionPackage) {
  return {
    schemaVersion: definitionPackage.schemaVersion,
    profile: definitionPackage.profile,
    runtimeReady: definitionPackage.runtimeReady,
    ...('compilationRootHash' in definitionPackage
      ? { compilationRootHash: definitionPackage.compilationRootHash }
      : {}),
    rootHash: definitionPackage.rootHash,
    workflow: {
      id: definitionPackage.workflow.id,
      modelDigest: definitionPackage.workflow.modelDigest,
      schemaVersion: definitionPackage.workflow.model.schemaVersion,
    },
    modules: definitionPackage.modules.map(module => ({
      name: module.name,
      exports: module.exports,
      bundle: module.bundle.digest,
      sourceMap: module.sourceMap.digest,
    })),
    artifacts: definitionPackage.artifacts.map(artifact => ({
      path: artifact.path,
      kind: artifact.kind,
      digest: artifact.digest,
    })),
  };
}

describe('WC0 frozen Workflow Call contracts', () => {
  test('lowers the reviewed parent and call-only child fixtures exactly', () => {
    for (const [sourceName, modelName] of [
      ['calculate-risk', 'calculate-risk.compiled.v10.json'],
      ['request-risk', 'request-risk.compiled.v8.json'],
    ] as const) {
      const compiled = compileWoml(document(sourceName));
      const expected = JSON.parse(
        readFileSync(resolve(fixtureDirectory, modelName), 'utf8')
      );
      expect(compiled).toEqual(expected);
      if (compiled.schemaVersion === 10) {
        expect(schemas.model(compiled), JSON.stringify(schemas.model.errors)).toBe(
          true
        );
      }
    }
  });

  test('validates every reviewed request, result, index, and routing fixture', async () => {
    const names = (await readdir(contractDirectory))
      .filter(name => name.endsWith('.json') && !name.endsWith('.invalid.json'))
      .sort();
    expect(names).toHaveLength(9);
    for (const name of names) {
      const fixture = await Bun.file(resolve(contractDirectory, name)).json();
      const validator = name.startsWith('index')
        ? schemas.index
        : name.startsWith('routing')
          ? schemas.routing
          : schemas.call;
      expect(
        validator(fixture),
        `${name}: ${JSON.stringify(validator.errors)}`
      ).toBe(true);
    }

    const missingResult = await Bun.file(
      resolve(contractDirectory, 'missing-result.invalid.json')
    ).json();
    expect(schemas.call(missingResult)).toBe(false);
  });

  test('validates truthful workflow-call ingress without a fake trigger', async () => {
    const fixture = await Bun.file(
      resolve(
        import.meta.dir,
        'fixtures/workflow-call-events/child-run-started.v9.json'
      )
    ).json();
    expect(
      schemas.event(fixture),
      JSON.stringify(schemas.event.errors)
    ).toBe(true);
    expect(fixture.data.triggerId).toBeUndefined();
    expect(fixture.data.ingress.kind).toBe('workflow_call');
  });

  test('keeps CLI call progress versioned and non-sensitive', () => {
    const progress = {
      contract: 'woml.workflow-call-progress',
      contractVersion: 1,
      type: 'call_admitted',
      parentRunId: 'run_parent',
      parentNodeId: 'calculateRisk',
      targetWorkflowId: 'calculate-risk',
      childRunId: 'run_child',
      duplicate: false,
      occurredAt: '2026-08-11T12:00:00.000Z',
    };
    expect(
      schemas.progress(progress),
      JSON.stringify(schemas.progress.errors)
    ).toBe(true);
    expect(schemas.progress({ ...progress, payload: { secret: true } })).toBe(
      false
    );
  });

  test('freezes call-only module packages as Definition Package v4 and v5', async () => {
    const sourcePath = resolve(
      fixtureDirectory,
      'calculate-risk-module.woml'
    );
    const sourceDocument = document('calculate-risk-module');
    const options = { sourcePath, projectRoot: repositoryRoot };
    const compiled = await buildWomlExecutableDefinitionPackage(
      sourceDocument,
      options
    );
    expect(compiled.schemaVersion).toBe(4);
    expect(compiled.profile).toBe(
      WOML_WORKFLOW_CALL_DEFINITION_PACKAGE_PROFILE
    );
    expect(compiled.workflow.model.schemaVersion).toBe(10);
    expect(
      schemas.packageV4(compiled),
      JSON.stringify(schemas.packageV4.errors)
    ).toBe(true);

    const runtime = await buildWomlRuntimeDefinitionPackage(
      sourceDocument,
      options
    );
    expect(runtime.schemaVersion).toBe(5);
    expect(runtime.profile).toBe(
      WOML_WORKFLOW_CALL_RUNTIME_DEFINITION_PACKAGE_PROFILE
    );
    expect(
      schemas.packageV5(runtime),
      JSON.stringify(schemas.packageV5.errors)
    ).toBe(true);
    const identity = JSON.parse(
      readFileSync(
        resolve(
          fixtureDirectory,
          'calculate-risk-module.package.identity.json'
        ),
        'utf8'
      )
    );
    expect(packageIdentity(compiled)).toEqual(identity.v4);
    expect(packageIdentity(runtime)).toEqual(identity.v5);
  });
});

describe('WC1 call-only frontend and workflow service analysis', () => {
  test('uses omitted triggers as the one call-only source shape', () => {
    const callOnly = `<woml><workflow id="worker"><steps>
      <step id="done"><script>return null;</script></step>
    </steps></workflow></woml>`;
    const compiled = compile(callOnly);
    expect(compiled.schemaVersion).toBe(10);
    expect(compiled.triggers).toEqual([]);

    const empty = `<woml><workflow id="worker"><triggers /><steps>
      <step id="done"><script>return null;</script></step>
    </steps></workflow></woml>`;
    expect(() => compile(empty)).toThrow(WomlValidationError);
    try {
      compile(empty);
    } catch (error) {
      expect((error as WomlValidationError).diagnostic.code).toBe(
        'WOML_TRIGGER_REQUIRED'
      );
    }
  });

  test('rejects Human Approval in a call-only v1 target', () => {
    const source = `<woml><workflow id="worker"><steps>
      <approval id="review"><when-approved /><when-rejected /></approval>
    </steps></workflow></woml>`;
    expect(() => compile(source)).toThrow(WomlCompileError);
    try {
      compile(source);
    } catch (error) {
      expect((error as WomlCompileError).diagnostic.code).toBe(
        'WOML_WORKFLOW_CALL_WAIT_UNSUPPORTED'
      );
    }
  });

  test('keeps existing triggered workflows on their previous model versions', () => {
    expect(
      compile(`<woml><workflow id="old-workflow">
        <triggers><manual id="start" /></triggers>
        <steps><step id="done"><script>return true;</script></step></steps>
      </workflow></woml>`).schemaVersion
    ).toBe(1);
  });

  test('accepts the v1 call and rejects statically invalid call shapes', () => {
    const accepted = analyzeWomlScript(
      `return services.workflows.call('calculate-risk', { customerId: '42' }, { name: 'risk', timeout: '30s' });`
    );
    expect(accepted.issue).toBeUndefined();
    expect(accepted.requiredServices).toContain('workflows');

    for (const [source, code] of [
      ['return services.workflows.call();', 'WOML_WORKFLOW_CALL_ARGUMENTS_INVALID'],
      [
        `return services.workflows.call('Bad Target', {});`,
        'WOML_WORKFLOW_TARGET_INVALID',
      ],
      [
        `return services.workflows.call('calculate-risk', []);`,
        'WOML_WORKFLOW_PAYLOAD_INVALID',
      ],
      [
        `return services.workflows.call('calculate-risk', {}, { cancel: true });`,
        'WOML_WORKFLOW_CALL_OPTION_UNKNOWN',
      ],
      [
        `return services.workflows.call('calculate-risk', {}, { timeout: '25h' });`,
        'WOML_WORKFLOW_CALL_OPTIONS_INVALID',
      ],
      [
        `return services.workflows.start('calculate-risk', {});`,
        'WOML_WORKFLOW_OPERATION_UNSUPPORTED',
      ],
    ] as const) {
      expect(analyzeWomlScript(source).issue?.code).toBe(code);
      expect(() => compile(scriptWorkflow(source))).toThrow(WomlValidationError);
    }
  });

  test('publishes workflows.call through automatic editor declarations', () => {
    const declarations = generateWomlEditorDeclarations([]);
    expect(declarations).toContain('readonly workflows');
    expect(declarations).toContain('readonly call');
    expect(declarations).toContain('workflowId: string');
    expect(declarations).toContain('WomlWorkflowCallOptions');
  });

  test('validateWoml accepts the reviewed call-only source', () => {
    expect(() => validateWoml(document('calculate-risk'))).not.toThrow();
  });
});

describe('WC6 Workflow Call composition contracts', () => {
  test('allows a workflow call from every current parent trigger type', () => {
    const triggers = [
      '<manual id="start" />',
      `<webhook id="start" path="/wc6" method="POST" auth="none"><schema>{"type":"object"}</schema></webhook>`,
      '<slack id="start" events="app-mention" channels="woml-testing" bot-token="{{secrets.SLACK_BOT_TOKEN}}" app-token="{{secrets.SLACK_APP_TOKEN}}" />',
      '<schedule id="start" cron="0 9 * * *" timezone="UTC" on-missed="skip" />',
      '<interval id="start" every="5s" on-missed="skip" />',
      `<event id="start" name="wc6.started" secret="{{secrets.EVENT_CONTROL_TOKEN}}"><schema>{"type":"object"}</schema></event>`,
    ];
    const handlers = triggers.map((trigger, index) => {
      const compiled = compile(`<woml><workflow id="caller-${index}">
        <triggers>${trigger}</triggers>
        <steps><step id="call"><script>
          return services.workflows.call('calculate-risk', {});
        </script></step></steps>
      </workflow></woml>`);
      return compiled.triggers[0]?.handler;
    });
    expect(handlers).toEqual([
      'trigger.manual',
      'trigger.webhook',
      'trigger.slack',
      'trigger.schedule',
      'trigger.interval',
      'trigger.event',
    ]);
  });
});
