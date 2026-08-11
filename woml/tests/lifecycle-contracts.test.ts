import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import Ajv2020 from 'ajv/dist/2020';

import {
  buildWomlExecutableDefinitionPackage,
  compileWoml,
  generateWomlLifecycleEditorDeclarations,
  parseWoml,
  WomlValidationError,
} from '../src';

const repositoryRoot = resolve(import.meta.dir, '../..');
const schemaRoot = resolve(repositoryRoot, 'docs/schemas');
const fixtureRoot = resolve(import.meta.dir, 'fixtures/lifecycle');

function source(name: string): string {
  return readFileSync(resolve(fixtureRoot, name), 'utf8');
}

function compile(value: string) {
  return compileWoml(parseWoml(value, { file: 'lifecycle.woml' }));
}

function error(value: string): WomlValidationError {
  try {
    compile(value);
  } catch (caught) {
    if (caught instanceof WomlValidationError) return caught;
    throw caught;
  }
  throw new Error('Expected lifecycle validation to fail.');
}

function workflow(
  lifecycle: string,
  steps = '<step id="a"><script>return { ok: true };</script></step>'
) {
  return `<woml><workflow id="lifecycle-test">${lifecycle}<triggers><manual id="start" /></triggers><steps>${steps}</steps></workflow></woml>`;
}

async function contractValidators() {
  const ajv = new Ajv2020({ strict: false, allErrors: true });
  ajv.addFormat(
    'date-time',
    /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z$/
  );
  const names = [
    ...Array.from(
      { length: 11 },
      (_, index) => `compiled-workflow-model.v${index + 1}.schema.json`
    ),
    ...Array.from(
      { length: 10 },
      (_, index) => `run-event.v${index + 1}.schema.json`
    ),
    ...Array.from(
      { length: 6 },
      (_, index) => `woml-definition-package.v${index + 1}.schema.json`
    ),
    'lifecycle-binding.v1.schema.json',
    'lifecycle-progress.v1.schema.json',
    'run-control.v1.schema.json',
    'run-inspection.v2.schema.json',
    'run-list.v1.schema.json',
    'notification-provider-host.v2.schema.json',
    'woml-template.v1.schema.json',
  ];
  for (const name of names) {
    ajv.addSchema(JSON.parse(readFileSync(resolve(schemaRoot, name), 'utf8')));
  }
  return ajv;
}

describe('LEC0 frozen lifecycle contracts', () => {
  test('validates every reviewed v1/v2/v10/v11 artifact and event fixture', async () => {
    const ajv = await contractValidators();
    const compiled = JSON.parse(source('lifecycle.compiled.v11.json'));
    const contracts = JSON.parse(source('contracts.v1.json'));
    const events = JSON.parse(source('events.v10.json')) as readonly unknown[];
    const validators = [
      ['https://cronflow.dev/schemas/compiled-workflow-model/v11', compiled],
      ['https://woml.dev/schemas/lifecycle-binding/v1', contracts.binding],
      ['https://woml.dev/schemas/lifecycle-progress/v1', contracts.progress],
      ['https://woml.dev/schemas/run-control/v1', contracts.controlRequest],
      ['https://woml.dev/schemas/run-control/v1', contracts.controlResult],
      ['https://woml.dev/schemas/run-list/v1', contracts.list],
      ['https://woml.dev/schemas/run-inspection/v2', contracts.inspection],
      [
        'https://cronflow.dev/schemas/notification-provider-host/v2',
        contracts.notification,
      ],
    ] as const;
    for (const [id, value] of validators) {
      const validate = ajv.getSchema(id)!;
      expect(validate(value), `${id}: ${JSON.stringify(validate.errors)}`).toBe(
        true
      );
    }
    const validateEvent = ajv.getSchema(
      'https://cronflow.dev/schemas/run-event/v10'
    )!;
    for (const event of events) {
      expect(validateEvent(event), JSON.stringify(validateEvent.errors)).toBe(
        true
      );
    }
  });

  test('freezes outcome-specific finalization and cancellation race order', () => {
    const sequences = JSON.parse(source('event-sequences.v10.json'));
    for (const name of ['success', 'failure', 'cancellation'] as const) {
      const sequence = sequences[name] as readonly string[];
      expect(sequence.at(-1)).toBe('run_finalized');
      expect(sequence.indexOf('run_outcome_decided')).toBeLessThan(
        sequence.lastIndexOf('lifecycle_hook_requested')
      );
    }
    expect(sequences.cancellation).toContain('run_cancellation_requested');
    expect(sequences.failure).not.toContain('run_cancellation_requested');
  });

  test('keeps secrets, context, messages, and approval capability out of durable fixtures', () => {
    const durable = `${source('events.v10.json')}\n${source('event-sequences.v10.json')}`;
    for (const forbidden of [
      'SLACK_BOT_TOKEN',
      'SLACK_APP_TOKEN',
      'decisionCapability',
      'context',
      'messageBody',
      'idempotencyKey',
    ]) {
      expect(durable).not.toContain(forbidden);
    }
  });

  test('rejects lifecycle handlers in the DAG and approval authority in informational delivery', async () => {
    const ajv = await contractValidators();
    const compiled = JSON.parse(source('lifecycle.compiled.v11.json'));
    compiled.graph.nodes.push({
      id: 'hiddenLifecycle',
      handler: 'runtime.lifecycle-script',
      inputs: { kind: 'object', fields: {} },
    });
    const validateModel = ajv.getSchema(
      'https://cronflow.dev/schemas/compiled-workflow-model/v11'
    )!;
    expect(validateModel(compiled)).toBe(false);

    const notification = JSON.parse(source('contracts.v1.json')).notification;
    notification.decisionCapability = 'forbidden';
    const validateNotification = ajv.getSchema(
      'https://cronflow.dev/schemas/notification-provider-host/v2'
    )!;
    expect(validateNotification(notification)).toBe(false);
  });
});

describe('LEC1 lifecycle frontend', () => {
  test('deep-equals the reviewed Model v11 fixture and keeps hooks outside the DAG', () => {
    const value = source('lifecycle.woml');
    const compiled = compileWoml(parseWoml(value, { file: 'lifecycle.woml' }));
    expect(compiled).toEqual(JSON.parse(source('lifecycle.compiled.v11.json')));
    expect(compiled.schemaVersion).toBe(11);
    expect(compiled.graph.nodes.map(node => node.id)).toEqual([
      'prepare',
      'finish',
    ]);
    if (compiled.schemaVersion === 11) {
      expect(compiled.lifecycle?.hooks).toHaveLength(9);
      expect(compiled.lifecycle?.hooks[1].stepIds).toEqual([
        'prepare',
        'finish',
      ]);
    }
  });

  test('validates lifecycle singleton, order, actions, and step filters precisely', () => {
    expect(error(source('invalid-order.woml')).diagnostic.code).toBe(
      'WOML_LIFECYCLE_ORDER_INVALID'
    );
    expect(error(workflow('<lifecycle />')).diagnostic.code).toBe(
      'WOML_LIFECYCLE_ACTION_REQUIRED'
    );
    expect(
      error(
        workflow(
          '<lifecycle><on-success><script>return;</script></on-success><on-start><script>return;</script></on-start></lifecycle>'
        )
      ).diagnostic.code
    ).toBe('WOML_LIFECYCLE_ORDER_INVALID');
    expect(
      error(
        workflow(
          '<lifecycle><on-start><script>return;</script></on-start><on-start><script>return;</script></on-start></lifecycle>'
        )
      ).diagnostic.code
    ).toBe('WOML_LIFECYCLE_DUPLICATE');
    expect(
      error(workflow('<lifecycle><on-success /></lifecycle>')).diagnostic.code
    ).toBe('WOML_LIFECYCLE_ACTION_REQUIRED');
    expect(
      error(
        workflow(
          '<lifecycle><on-success><step id="bad"><script>return;</script></step></on-success></lifecycle>'
        )
      ).diagnostic.code
    ).toBe('WOML_LIFECYCLE_ACTION_INVALID');
    expect(
      error(
        workflow(
          '<lifecycle><on-success steps="a"><script>return;</script></on-success></lifecycle>'
        )
      ).diagnostic.code
    ).toBe('WOML_LIFECYCLE_STEP_FILTER_INVALID');
    expect(
      error(
        workflow(
          '<lifecycle><on-step-success steps="missing"><script>return;</script></on-step-success></lifecycle>'
        )
      ).diagnostic.code
    ).toBe('WOML_LIFECYCLE_STEP_UNKNOWN');
    expect(
      error(
        workflow(
          '<lifecycle><on-step-success steps="a a"><script>return;</script></on-step-success></lifecycle>'
        )
      ).diagnostic.code
    ).toBe('WOML_LIFECYCLE_STEP_FILTER_INVALID');
  });

  test('discovers executable step IDs nested in branch, parallel, and approval structures', () => {
    const steps = `
      <step id="decision"><script>return { yes: true };</script></step>
      <branch id="route">
        <when test="{{context.steps.decision.yes}}"><step id="branchStep"><script>return 1;</script></step><result value="{{context.steps.branchStep}}" /></when>
        <otherwise><step id="fallbackStep"><script>return 0;</script></step><result value="{{context.steps.fallbackStep}}" /></otherwise>
      </branch>
      <parallel id="checks"><step id="parallelStep"><script>return true;</script></step></parallel>
      <approval id="review"><when-approved><step id="approvedStep"><script>return true;</script></step></when-approved><when-rejected><step id="rejectedStep"><script>return false;</script></step></when-rejected></approval>
      <step id="finish"><script>return context.steps.route;</script></step>`;
    const compiled = compile(
      workflow(
        '<lifecycle><on-step-complete steps="branchStep fallbackStep parallelStep approvedStep rejectedStep"><script>return;</script></on-step-complete></lifecycle>',
        steps
      )
    );
    expect(compiled.schemaVersion).toBe(11);
    if (compiled.schemaVersion === 11) {
      expect(compiled.lifecycle?.hooks[0].stepIds).toEqual([
        'branchStep',
        'fallbackStep',
        'parallelStep',
        'approvedStep',
        'rejectedStep',
      ]);
    }
  });

  test('separates informational Slack from triggers and approval notifications', () => {
    expect(error(source('invalid-template.woml')).diagnostic.code).toBe(
      'WOML_SECRET_SINK_UNSUPPORTED'
    );
    const missingMessage = workflow(
      `<lifecycle><on-failure><notify><slack channels="#ops" bot-token="{{secrets.BOT}}" app-token="{{secrets.APP}}" /></notify></on-failure></lifecycle>`
    );
    expect(error(missingMessage).diagnostic.code).toBe(
      'WOML_SLACK_ATTRIBUTE_REQUIRED'
    );
    const malformed = workflow(
      `<lifecycle><on-failure><notify><slack channels="#ops" message="Failure {{lifecycle.failure.code}" bot-token="{{secrets.BOT}}" app-token="{{secrets.APP}}" /></notify></on-failure></lifecycle>`
    );
    expect(error(malformed).diagnostic.code).toBe(
      'WOML_LIFECYCLE_TEMPLATE_INVALID'
    );
    const secretMessage = workflow(
      `<lifecycle><on-failure><notify><slack channels="#ops" message="{{secrets.PRIVATE}}" bot-token="{{secrets.BOT}}" app-token="{{secrets.APP}}" /></notify></on-failure></lifecycle>`
    );
    expect(error(secretMessage).diagnostic.code).toBe(
      'WOML_SECRET_SINK_UNSUPPORTED'
    );
    const approvalMessage = workflow(
      '',
      `<approval id="review"><notify><slack channels="#ops" message="Not allowed" bot-token="{{secrets.BOT}}" app-token="{{secrets.APP}}" /></notify><when-approved /><when-rejected /></approval>`
    );
    expect(error(approvalMessage).diagnostic.code).toBe(
      'WOML_SLACK_UNKNOWN_ATTRIBUTE'
    );
    const triggerMessage = `<woml><workflow id="slack-trigger-message"><triggers><slack id="message" events="app-mention" message="Not allowed" bot-token="{{secrets.BOT}}" app-token="{{secrets.APP}}" /></triggers><steps><step id="a"><script>return true;</script></step></steps></workflow></woml>`;
    expect(error(triggerMessage).diagnostic.code).toBe(
      'WOML_SLACK_UNKNOWN_ATTRIBUTE'
    );
  });

  test('keeps lifecycle unavailable in normal scripts and read-only in lifecycle scripts', () => {
    const unavailable = workflow(
      '',
      '<step id="a"><script>return lifecycle.event;</script></step>'
    );
    expect(error(unavailable).diagnostic.code).toBe(
      'WOML_LIFECYCLE_BINDING_UNAVAILABLE'
    );
    const readOnly = workflow(
      '<lifecycle><on-start><script>lifecycle.workflow.id = "other";</script></on-start></lifecycle>'
    );
    expect(error(readOnly).diagnostic.code).toBe(
      'WOML_LIFECYCLE_BINDING_READ_ONLY'
    );
  });

  test('discovers module usage in lifecycle scripts and emits Definition Package v6', async () => {
    const path = resolve(fixtureRoot, 'lifecycle-module.woml');
    const document = parseWoml(readFileSync(path, 'utf8'), { file: path });
    const definitionPackage = await buildWomlExecutableDefinitionPackage(
      document,
      {
        sourcePath: path,
        projectRoot: resolve(import.meta.dir, 'fixtures'),
      }
    );
    expect(definitionPackage.schemaVersion).toBe(6);
    expect(definitionPackage.workflow.model.schemaVersion).toBe(11);
    const ajv = await contractValidators();
    const validate = ajv.getSchema(
      'https://woml.dev/schemas/woml-definition-package.v6.schema.json'
    )!;
    expect(validate(definitionPackage), JSON.stringify(validate.errors)).toBe(
      true
    );
  });

  test('publishes the deeply read-only lifecycle editor binding', () => {
    const declarations = generateWomlLifecycleEditorDeclarations();
    expect(declarations).toContain(
      'declare const lifecycle: Readonly<WomlLifecycleBinding>'
    );
    expect(declarations).toContain("| 'run_complete'");
    expect(declarations).not.toContain('context.run');
  });
});
