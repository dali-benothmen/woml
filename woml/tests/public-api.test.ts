import { describe, expect, test } from 'bun:test';

import * as woml from '../src';

describe('WOML frontend public API', () => {
  test('exports the validation pass independently from model lowering', () => {
    expect(woml.validateWoml).toBeFunction();
  });

  test('does not export the retired TypeScript execution surface', () => {
    expect(woml).not.toHaveProperty('executeWorkflow');
    expect(woml).not.toHaveProperty('createRuntimeHandlerRegistry');
    expect(woml).not.toHaveProperty('HandlerRegistry');
    expect(woml).not.toHaveProperty('runScriptInWorker');
    expect(woml).not.toHaveProperty('WorkflowExecutionError');
  });
});
