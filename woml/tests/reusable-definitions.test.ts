import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import {
  assertWomlDocumentRunnable,
  generateWomlReusableCustomData,
  inspectWomlDocument,
  parseWoml,
  resolveWomlReusableDefinitionGraph,
  validateResolvedReusableWorkflow,
  validateWoml,
  WomlCompileError,
  WomlValidationError,
} from '../src';

const fixtureRoot = resolve(import.meta.dir, 'fixtures/reusable-definitions');

function fixture(name: string) {
  const path = resolve(fixtureRoot, name);
  return parseWoml(readFileSync(path, 'utf8'), { file: path });
}

function validationCode(source: string): string {
  try {
    validateWoml(parseWoml(source, { file: 'invalid-definition.woml' }));
  } catch (error) {
    expect(error).toBeInstanceOf(WomlValidationError);
    return (error as WomlValidationError).diagnostic.code;
  }
  throw new Error('Expected validation to fail.');
}

describe('reusable WOML document contracts', () => {
  test('classifies workflow, reusable step, and notification provider documents', () => {
    expect(inspectWomlDocument(fixture('workflow.woml')).kind).toBe('workflow');
    const step = inspectWomlDocument(fixture('calculate-discount.woml'));
    expect(step.kind).toBe('reusable-step');
    expect(step.props.map(prop => [prop.name, prop.bindingName])).toEqual([
      ['price', 'price'],
      ['percentage', 'percentage'],
    ]);
    const provider = inspectWomlDocument(fixture('telegram.woml'));
    expect(provider.kind).toBe('notification-provider');
    expect(provider.props[0]).toMatchObject({
      name: 'bot-token',
      bindingName: 'botToken',
      required: true,
      secret: true,
    });
  });

  test('accepts reusable definitions through public validation but keeps them non-runnable', () => {
    expect(() => validateWoml(fixture('calculate-discount.woml'))).not.toThrow();
    expect(() => assertWomlDocumentRunnable(fixture('calculate-discount.woml'))).toThrow(
      WomlCompileError
    );
    try {
      assertWomlDocumentRunnable(fixture('calculate-discount.woml'));
    } catch (error) {
      expect((error as WomlCompileError).diagnostic.code).toBe(
        'WOML_DEFINITION_NOT_RUNNABLE'
      );
    }
  });

  test('forbids props in every workflow document and inside definition tags', () => {
    expect(
      validationCode(`<woml><workflow id="bad"><props><prop name="value" /></props><steps><step id="done"><script>return true;</script></step></steps></workflow></woml>`)
    ).toBe('WOML_PROPS_WORKFLOW_FORBIDDEN');
    expect(
      validationCode(`<woml><props><prop name="value" /></props><workflow id="bad"><steps><step id="done"><script>return true;</script></step></steps></workflow></woml>`)
    ).toBe('WOML_PROPS_WORKFLOW_FORBIDDEN');
    expect(
      validationCode(`<woml><step><props><prop name="value" /></props><script>return true;</script></step></woml>`)
    ).toBe('WOML_REUSABLE_STEP_OPERATION_REQUIRED');
  });

  test('freezes prop names, booleans, lifecycle hooks, and provider kinds', () => {
    expect(
      validationCode(`<woml><props><prop name="BadName" /></props><step><script>return true;</script></step></woml>`)
    ).toBe('WOML_PROP_NAME_INVALID');
    expect(
      validationCode(`<woml><props><prop name="value" required="yes" /></props><step><script>return true;</script></step></woml>`)
    ).toBe('WOML_PROP_BOOLEAN_INVALID');
    expect(
      validationCode(`<woml><provider kind="trigger"><script>return true;</script></provider></woml>`)
    ).toBe('WOML_PROVIDER_KIND_UNSUPPORTED');
    expect(
      validationCode(`<woml><step><script>return true;</script></step><lifecycle><on-step-success><script>return;</script></on-step-success></lifecycle></woml>`)
    ).toBe('WOML_REUSABLE_STEP_HOOK_UNSUPPORTED');
  });
});

describe('reusable definition graph resolution', () => {
  test('resolves and validates custom step/provider props deterministically', () => {
    const workflow = fixture('workflow.woml');
    const first = resolveWomlReusableDefinitionGraph(workflow, {
      sourcePath: resolve(fixtureRoot, 'workflow.woml'),
      projectRoot: fixtureRoot,
    });
    const second = resolveWomlReusableDefinitionGraph(workflow, {
      sourcePath: resolve(fixtureRoot, 'workflow.woml'),
      projectRoot: fixtureRoot,
    });
    expect(second).toEqual(first);
    expect(first.root.kind).toBe('workflow');
    expect(first.definitions.map(item => [item.alias, item.kind])).toEqual([
      ['calculate-discount', 'reusable-step'],
      ['telegram', 'notification-provider'],
    ]);
    expect(first.sources.map(item => item.path)).toEqual([
      'calculate-discount.woml',
      'pricing-helper.ts',
      'pricing.ts',
      'telegram.woml',
      'workflow.woml',
    ]);
    expect(first.rootHash).toMatch(/^sha256:[0-9a-f]{64}$/);
    const customData = JSON.parse(generateWomlReusableCustomData(first));
    expect(customData.tags.map((tag: { name: string }) => tag.name)).toEqual([
      'calculate-discount',
      'telegram',
    ]);
  });

  test('rejects missing, unknown, and unsafe secret prop bindings', () => {
    const workflow = (usage: string) => parseWoml(`<woml>
  <imports><module name="telegram" from="./telegram.woml" /></imports>
  <workflow id="prop-test"><steps>
    <step id="before"><script>return true;</script></step>
    <approval id="review"><notify>${usage}</notify><when-approved></when-approved><when-rejected></when-rejected></approval>
    <step id="after"><script>return true;</script></step>
  </steps></workflow>
</woml>`, { file: resolve(fixtureRoot, 'prop-test.woml') });
    const code = (usage: string) => {
      try {
        resolveWomlReusableDefinitionGraph(workflow(usage), {
          sourcePath: resolve(fixtureRoot, 'workflow.woml'),
          projectRoot: fixtureRoot,
        });
      } catch (error) {
        expect(error).toBeInstanceOf(WomlValidationError);
        return (error as WomlValidationError).diagnostic.code;
      }
      throw new Error('Expected prop validation failure.');
    };
    expect(code('<telegram chat-id="team" />')).toBe('WOML_REUSABLE_PROP_REQUIRED');
    expect(
      code('<telegram bot-token="literal" chat-id="team" />')
    ).toBe('WOML_REUSABLE_SECRET_PROP_INVALID');
    expect(
      code('<telegram bot-token="{{secrets.TOKEN}}" chat-id="team" extra="x" />')
    ).toBe('WOML_REUSABLE_PROP_UNKNOWN');
  });

  test('rejects workflow imports and recursive reusable import cycles', () => {
    const workflowImport = parseWoml(`<woml>
  <imports><module name="child-workflow" from="./workflow.woml" /></imports>
  <workflow id="parent"><steps><step id="done"><script>return true;</script></step></steps></workflow>
</woml>`, { file: resolve(fixtureRoot, 'parent.woml') });
    expect(() =>
      resolveWomlReusableDefinitionGraph(workflowImport, {
        sourcePath: resolve(fixtureRoot, 'workflow.woml'),
        projectRoot: fixtureRoot,
      })
    ).toThrow(WomlCompileError);

    const cycleRoot = parseWoml(`<woml>
  <imports><module name="cycle-a" from="./cycle-a.woml" /></imports>
  <workflow id="cycle-root"><steps><cycle-a id="runCycle" /></steps></workflow>
</woml>`, { file: resolve(fixtureRoot, 'cycle-root.woml') });
    try {
      resolveWomlReusableDefinitionGraph(cycleRoot, {
        sourcePath: resolve(fixtureRoot, 'workflow.woml'),
        projectRoot: fixtureRoot,
      });
      throw new Error('Expected a reusable import cycle.');
    } catch (error) {
      expect(error).toBeInstanceOf(WomlCompileError);
      expect((error as WomlCompileError).diagnostic.code).toBe(
        'WOML_REUSABLE_IMPORT_CYCLE'
      );
    }
  });

  test('keeps ordinary workflow grammar validation active beside custom tags', () => {
    const path = resolve(fixtureRoot, 'workflow.woml');
    const document = parseWoml(`<woml>
  <imports><module name="calculate-discount" from="./calculate-discount.woml" /></imports>
  <workflow id="invalid-workflow"><steps>
    <calculate-discount id="discount" price="100" percentage="20" />
    <step><script>return true;</script></step>
  </steps></workflow>
</woml>`, { file: path });
    const graph = resolveWomlReusableDefinitionGraph(document, {
      sourcePath: path,
      projectRoot: fixtureRoot,
    });
    try {
      validateResolvedReusableWorkflow(document, graph);
      throw new Error('Expected ordinary workflow validation to fail.');
    } catch (error) {
      expect(error).toBeInstanceOf(WomlValidationError);
      expect((error as WomlValidationError).diagnostic.code).toBe(
        'WOML_MISSING_ATTRIBUTE'
      );
    }
  });

  test('requires a message for informational custom-provider usage', () => {
    try {
      resolveWomlReusableDefinitionGraph(fixture('notify-with-telegram.woml'), {
        sourcePath: resolve(fixtureRoot, 'notify-with-telegram.woml'),
        projectRoot: fixtureRoot,
      });
      throw new Error('Expected informational provider validation to fail.');
    } catch (error) {
      expect(error).toBeInstanceOf(WomlValidationError);
      expect((error as WomlValidationError).diagnostic.code).toBe(
        'WOML_REUSABLE_PROVIDER_MESSAGE_REQUIRED'
      );
    }
  });
});
