import { WorkflowDefinition } from '../workflow';
import { loadCoreModule } from '../utils/core-resolver';

const { core } = loadCoreModule();

let getCurrentStateFunction: () => any = () => ({});

export function setRustIntegrationState(getCurrentState: () => any): void {
  getCurrentStateFunction = getCurrentState;
}

export async function registerWorkflowWithRust(
  workflow: WorkflowDefinition
): Promise<void> {
  if (!core) {
    return;
  }

  const currentState = getCurrentStateFunction();

  try {
    const rustFormat = convertToRustFormat(workflow);
    const workflowJson = JSON.stringify(rustFormat);

    const result = core.registerWorkflow(workflowJson, currentState.dbPath);

    if (!result.success) {
      throw new Error(`Failed to register workflow: ${result.message}`);
    }
  } catch (error) {
    throw error;
  }
}

export function convertToRustFormat(workflow: WorkflowDefinition): any {
  return {
    id: workflow.id,
    name: workflow.name || workflow.id,
    description: workflow.description || '',
    concurrency: workflow.concurrency || null,
    steps: workflow.steps.map(step => ({
      id: step.id,
      name: step.name,
      title: step.title,
      description: step.description,
      action: step.handler.toString(),
      type: step.type,
      handler: step.handler.toString(),
      timeout: step.options?.timeout
        ? typeof step.options.timeout === 'string'
          ? parseDuration(step.options.timeout)
          : step.options.timeout
        : 30000,
      retry: step.options?.retry
        ? {
            max_attempts: step.options.retry.attempts,
            backoff_ms: step.options.retry.backoff
              ? parseDuration(step.options.retry.backoff.delay)
              : 1000,
          }
        : { max_attempts: 1, backoff_ms: 1000 },
      depends_on: [],
      is_control_flow: false,
      condition_type: null,
      condition_expression: null,
      control_flow_block: null,
      parallel: null,
      parallel_group_id: null,
      parallel_step_count: null,
      race: null,
      for_each: null,
      pause: null,
    })),
    triggers: workflow.triggers.map(trigger => {
      if (trigger.type === 'webhook') {
        return {
          Webhook: {
            path: trigger.path,
            method: trigger.options?.method || 'POST',
          },
        };
      } else if (trigger.type === 'schedule') {
        return 'Manual';
      } else if (trigger.type === 'event') {
        return 'Manual';
      } else {
        return 'Manual';
      }
    }),
    created_at: workflow.created_at.toISOString(),
    updated_at: workflow.updated_at.toISOString(),
  };
}

export function parseDuration(duration: string | number): number {
  if (typeof duration === 'number') {
    return duration;
  }

  const match = duration.match(/^(\d+)(ms|s|m|h|d)$/);
  if (!match) {
    throw new Error(`Invalid duration format: ${duration}`);
  }

  const [, value, unit] = match;
  const numValue = parseInt(value, 10);

  switch (unit) {
    case 'ms':
      return numValue;
    case 's':
      return numValue * 1000;
    case 'm':
      return numValue * 60 * 1000;
    case 'h':
      return numValue * 60 * 60 * 1000;
    case 'd':
      return numValue * 24 * 60 * 60 * 1000;
    default:
      throw new Error(`Unknown duration unit: ${unit}`);
  }
}

export function isRustCoreAvailable(): boolean {
  return core !== null;
}

export function getCoreStatus(): {
  available: boolean;
  path?: string;
  error?: string;
} {
  if (core) {
    return { available: true };
  } else {
    return {
      available: false,
      error: 'Core not loaded - running in simulation mode',
    };
  }
}
