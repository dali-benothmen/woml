import { describe, expect, test } from 'bun:test';

import * as woml from '../src';

describe('WOML frontend public API', () => {
  test('exports the validation pass independently from model lowering', () => {
    expect(woml.validateWoml).toBeFunction();
  });

  test('exports deterministic module inspection and compilation without a runtime loader', () => {
    expect(woml.buildWomlDefinitionPackage).toBeFunction();
    expect(woml.buildWomlExecutableDefinitionPackage).toBeFunction();
    expect(woml.WOML_EXECUTABLE_DEFINITION_PACKAGE_PROFILE).toBe(
      'woml.definition-package/v2'
    );
    expect(woml).not.toHaveProperty('loadWomlModuleArtifacts');
  });

  test('does not export the retired TypeScript execution surface', () => {
    expect(woml).not.toHaveProperty('executeWorkflow');
    expect(woml).not.toHaveProperty('createRuntimeHandlerRegistry');
    expect(woml).not.toHaveProperty('HandlerRegistry');
    expect(woml).not.toHaveProperty('runScriptInWorker');
    expect(woml).not.toHaveProperty('WorkflowExecutionError');
  });
});
