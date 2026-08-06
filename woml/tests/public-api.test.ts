import { describe, expect, test } from 'bun:test';

import * as woml from '../src';

describe('WOML frontend public API', () => {
  test('does not export the retired TypeScript execution surface', () => {
    expect(woml).not.toHaveProperty('executeWorkflow');
    expect(woml).not.toHaveProperty('createRuntimeHandlerRegistry');
    expect(woml).not.toHaveProperty('HandlerRegistry');
    expect(woml).not.toHaveProperty('runScriptInWorker');
    expect(woml).not.toHaveProperty('WorkflowExecutionError');
  });
});
