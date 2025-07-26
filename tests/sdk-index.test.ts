import { describe, it, expect } from 'vitest';
import { cronflow, VERSION } from '../sdk/index';

describe('Cronflow SDK', () => {
  it('should export correct version', () => {
    expect(VERSION).toBe('0.1.0');
  });

  it('should initialize cronflow singleton', () => {
    expect(cronflow).toBeDefined();
    expect(typeof cronflow.define).toBe('function');
    expect(typeof cronflow.start).toBe('function');
  });

  it('should define a workflow', () => {
    const workflow = cronflow.define({
      id: 'test',
      name: 'Test Workflow',
      description: 'A test workflow',
    });

    expect(workflow).toBeDefined();
    expect(workflow.getId()).toBe('test');
  });

  it('should get defined workflows', () => {
    const workflows = cronflow.getWorkflows();
    expect(Array.isArray(workflows)).toBe(true);
  });
});
