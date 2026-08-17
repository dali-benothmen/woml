import { describe, expect, test } from 'bun:test';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import {
  compileWoml,
  generateWomlEditorDeclarations,
  inspectWomlModuleServiceUsage,
  inspectWomlModuleUsage,
  parseWoml,
  WomlDiagnosticError,
} from '../src';

describe('Durable User State authoring surface', () => {
  test('discovers services.state in steps and lifecycle scripts', () => {
    const document = parseWoml(`<woml>
<workflow id="state-authoring" name="State authoring" version="1.0.0">
  <lifecycle>
    <on-complete><script>await services.state.has('completed');</script></on-complete>
  </lifecycle>
  <triggers><manual id="start" /></triggers>
  <steps><step id="remember"><script>
    return await services.state.get('conversation:C123');
  </script></step></steps>
</workflow>
</woml>`, { file: 'state-authoring.woml' });
    expect(inspectWomlModuleUsage(document).referencedServices).toEqual(['state']);
    expect(compileWoml(document).schemaVersion).toBe(11);
  });

  test('reserves state so a local module cannot shadow the built-in service', () => {
    const document = parseWoml(`<woml>
<imports><module name="state" from="./state.ts" /></imports>
<workflow id="state-shadow"><triggers><manual id="start" /></triggers>
<steps><step id="done"><script>return true;</script></step></steps></workflow>
</woml>`, { file: 'state-shadow.woml' });
    expect(() => compileWoml(document)).toThrow(WomlDiagnosticError);
    try {
      compileWoml(document);
    } catch (error) {
      expect((error as WomlDiagnosticError).diagnostic.code).toBe('WOML_MODULE_ALIAS_RESERVED');
    }
  });

  test('discovers services.state inside imported JavaScript and TypeScript sources', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-ds1-modules-'));
    try {
      const workflowPath = join(directory, 'workflow.woml');
      await Bun.write(
        workflowPath,
        `<woml><imports><module name="memory" from="./memory.ts" /></imports>
<workflow id="module-state"><triggers><manual id="start" /></triggers>
<steps><step id="read"><script>return services.memory.read();</script></step></steps>
</workflow></woml>`
      );
      await Bun.write(
        join(directory, 'memory.ts'),
        `export async function read() { return services.state.get('module-key'); }`
      );
      const document = parseWoml(await Bun.file(workflowPath).text(), { file: workflowPath });
      expect(
        inspectWomlModuleServiceUsage(document, {
          sourcePath: workflowPath,
          projectRoot: directory,
        })
      ).toEqual({
        referencedServices: ['state'],
        durableStateSources: ['memory.ts'],
      });
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  test('rejects dynamic state aliases in WOML scripts and imported modules', async () => {
    const dynamicScript = parseWoml(`<woml><workflow id="dynamic-state">
<triggers><manual id="start" /></triggers><steps><step id="read"><script>
return services['state'].get('key');
</script></step></steps></workflow></woml>`, { file: 'dynamic-state.woml' });
    expect(() => compileWoml(dynamicScript)).toThrow(
      'Computed service access is not executable'
    );

    const directory = await mkdtemp(join(tmpdir(), 'woml-ds1-dynamic-module-'));
    try {
      const workflowPath = join(directory, 'workflow.woml');
      await Bun.write(
        workflowPath,
        `<woml><imports><module name="memory" from="./memory.js" /></imports>
<workflow id="dynamic-module"><triggers><manual id="start" /></triggers>
<steps><step id="read"><script>return services.memory.read();</script></step></steps>
</workflow></woml>`
      );
      await Bun.write(
        join(directory, 'memory.js'),
        `export async function read() { return services['state'].get('key'); }`
      );
      const document = parseWoml(await Bun.file(workflowPath).text(), { file: workflowPath });
      expect(() =>
        inspectWomlModuleServiceUsage(document, {
          sourcePath: workflowPath,
          projectRoot: directory,
        })
      ).toThrow('Computed service access is not supported');
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  test('generates the reviewed StateService fixture without widening context or secrets', async () => {
    const declarations = generateWomlEditorDeclarations([]);
    const reviewed = await Bun.file(
      join(import.meta.dir, 'fixtures/durable-state/state-service.generated.d.ts')
    ).text();
    expect(declarations).toContain(reviewed.trim());
    expect(declarations).toContain('readonly state: WomlStateService;');
    expect(declarations).toContain('readonly ifVersion?: number;');
    expect(declarations).toContain('declare const services:');
    expect(declarations).not.toContain('declare const context');
    expect(declarations).not.toContain('declare const secrets');
  });
});
