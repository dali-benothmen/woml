import * as cronflowFunctions from './cronflow';

export const cronflow = {
  define: cronflowFunctions.define,

  start: cronflowFunctions.start,

  stop: cronflowFunctions.stop,

  trigger: cronflowFunctions.trigger,

  inspect: cronflowFunctions.inspect,

  executeManualTrigger: cronflowFunctions.executeManualTrigger,

  executeWebhookTrigger: cronflowFunctions.executeWebhookTrigger,

  executeScheduleTrigger: cronflowFunctions.executeScheduleTrigger,

  getTriggerStats: cronflowFunctions.getTriggerStats,

  getWorkflowTriggers: cronflowFunctions.getWorkflowTriggers,

  unregisterWorkflowTriggers: cronflowFunctions.unregisterWorkflowTriggers,

  getScheduleTriggers: cronflowFunctions.getScheduleTriggers,

  getWorkflows: cronflowFunctions.getWorkflows,

  getWorkflow: cronflowFunctions.getWorkflow,

  getEngineState: cronflowFunctions.getEngineState,

  isRustCoreAvailable: cronflowFunctions.isRustCoreAvailable,

  replay: cronflowFunctions.replay,

  resume: cronflowFunctions.resume,

  cancelRun: cronflowFunctions.cancelRun,

  publishEvent: cronflowFunctions.publishEvent,

  executeStep: cronflowFunctions.executeStep,
  executeStepFunction: cronflowFunctions.executeStepFunction,
  executeJobFunction: cronflowFunctions.executeJobFunction,

  // State management functions
  getGlobalState: cronflowFunctions.getGlobalState,
  setGlobalState: cronflowFunctions.setGlobalState,
  incrGlobalState: cronflowFunctions.incrGlobalState,
  deleteGlobalState: cronflowFunctions.deleteGlobalState,
  getWorkflowState: cronflowFunctions.getWorkflowState,
  setWorkflowState: cronflowFunctions.setWorkflowState,
  incrWorkflowState: cronflowFunctions.incrWorkflowState,
  deleteWorkflowState: cronflowFunctions.deleteWorkflowState,
  getStateStats: cronflowFunctions.getStateStats,
  cleanupExpiredState: cronflowFunctions.cleanupExpiredState,

  // Performance benchmarking
  benchmark: cronflowFunctions.benchmark,
};

// Export types
export type { WebhookServerConfig, StartOptions } from './cronflow';

export type { WorkflowDefinition } from './workflow/types';
export type { Context } from './workflow/types';
export type { StepDefinition, StepOptions } from './workflow/types';
export type { TriggerDefinition } from './workflow/types';
export type { BenchmarkOptions, BenchmarkResult } from './cronflow';

export { WorkflowDefinitionSchema } from './workflow/validation';

export * from './workflow';

export * from './state';

export * from './testing';

export { createValidContext } from './cronflow';
