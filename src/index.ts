/**
 * Node-Cronflow Main Entry Point
 *
 * This is the main entry point for the node-cronflow package.
 * It exports the SDK and services for use by consumers.
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

// Export Services
export {
  defineService,
  getServiceDefinition,
  listServiceDefinitions,
} from '../services/index';
export type { ServiceDefinition, ConfiguredService } from '../services/index';

// Default export for backward compatibility
export { cronflow as default } from '../sdk/index';
