/**
 * Cronflow Main Entry Point
 *
 * This is the main entry point for the cronflow package.
 * It exports the SDK for use by consumers.
 */

// Export SDK (main exports)
export { cronflow, VERSION } from '../sdk/index';
export type {
  WorkflowDefinition,
  StepDefinition,
  RetryConfig,
  TriggerDefinition,
  WorkflowInstance,
} from '../sdk/index';

// Default export for backward compatibility
export { cronflow as default } from '../sdk/index';
