import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import {
  compileWoml,
  inspectCompiledWorkflowGraph,
  parseWoml,
  WomlCompileError,
  WomlValidationError,
} from '../src';

const examplePath = resolve(
  import.meta.dir,
  '../../examples/switchWorkflow.woml'
);

function workflow(flow: string): string {
  return `<woml>
    <workflow id="switch-test" version="1.0.0">
      <triggers><manual id="start" /></triggers>
      <steps>
        <step id="load"><script>return { value: "one" };</script></step>
        ${flow}
        <step id="finish"><script>return context.steps.load;</script></step>
      </steps>
    </workflow>
  </woml>`;
}

describe('switch control flow', () => {
  test('lowers exact string cases and a default to Model v14 choice authority', () => {
    const compiled = compileWoml(
      parseWoml(readFileSync(examplePath, 'utf8'), { file: examplePath })
    );
    expect(compiled.schemaVersion).toBe(14);
    if (compiled.schemaVersion !== 14) throw new Error('Expected Model v14.');
    expect(inspectCompiledWorkflowGraph(compiled.graph)).toEqual([]);

    const choice = compiled.graph.choices[0];
    expect(choice.stringSelector).toEqual({
      kind: 'contextReference',
      path: ['steps', 'loadOrder', 'provider'],
    });
    expect(choice.stringCases?.map(item => item.value)).toEqual([
      'slack',
      'email',
    ]);
    expect(choice.defaultArmId).toBe(choice.armIds.at(-1));
    expect(choice.resultNodeId).toBe('delivery');
    expect(
      compiled.graph.nodes.find(node => node.id === 'delivery')?.handler
    ).toBe('engine.choice-result');
  });

  test('accepts a control-only switch and preserves no public switch output', () => {
    const compiled = compileWoml(
      parseWoml(
        workflow(`<switch value="{{context.steps.load.value}}">
          <case value="one"><step id="one"><script>return 1;</script></step></case>
          <default><step id="other"><script>return 0;</script></step></default>
        </switch>`)
      )
    );
    expect(compiled.schemaVersion).toBe(14);
    if (compiled.schemaVersion !== 14) throw new Error('Expected Model v14.');
    expect(compiled.graph.choices[0].resultNodeId).toBeUndefined();
    expect(
      compiled.graph.nodes.some(node => node.handler === 'engine.choice-result')
    ).toBe(false);
  });

  test('rejects duplicate cases and invalid result profiles with stable diagnostics', () => {
    expect(() =>
      compileWoml(
        parseWoml(
          workflow(`<switch value="{{context.steps.load.value}}">
            <case value="one"><step id="one"><script>return 1;</script></step></case>
            <case value="one"><step id="again"><script>return 2;</script></step></case>
            <default><step id="other"><script>return 0;</script></step></default>
          </switch>`)
        )
      )
    ).toThrow(WomlValidationError);

    try {
      compileWoml(
        parseWoml(
          workflow(`<switch id="selected" value="{{context.steps.load.value}}">
            <case value="one"><step id="one"><script>return 1;</script></step></case>
            <default><step id="other"><script>return 0;</script></step><result value="{{context.steps.other}}" /></default>
          </switch>`)
        )
      );
      throw new Error('Expected validation to fail.');
    } catch (error) {
      expect(error).toBeInstanceOf(WomlValidationError);
      expect((error as WomlValidationError).diagnostic.code).toBe(
        'WOML_SWITCH_RESULT_REQUIRED'
      );
    }
  });

  test('rejects selector references that are not available before the switch', () => {
    expect(() =>
      compileWoml(
        parseWoml(
          workflow(`<switch value="{{context.steps.future.value}}">
            <case value="one"><step id="one"><script>return 1;</script></step></case>
            <default><step id="future"><script>return { value: "later" };</script></step></default>
          </switch>`)
        )
      )
    ).toThrow(WomlCompileError);
  });
});
