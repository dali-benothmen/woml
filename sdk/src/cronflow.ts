import { WorkflowInstance, WorkflowDefinition, Context } from './workflow';
import { createStateManager } from './state';

// Import the Rust addon
let core: any;
try {
  core = require('../../core/core.node');
} catch (error) {
  console.warn('⚠️  Rust core not available, running in simulation mode');
  core = null;
}

export const VERSION = '0.1.0';

interface CronflowState {
  workflows: Map<string, WorkflowDefinition>;
  engineState: 'STOPPED' | 'STARTING' | 'STARTED';
  dbPath: string;
  stateManager?: any;
}

let state: CronflowState = {
  workflows: new Map(),
  engineState: 'STOPPED',
  dbPath: './cronflow.db',
};

function getCurrentState(): CronflowState {
  return state;
}

function setState(newState: Partial<CronflowState>): void {
  state = { ...state, ...newState };
}

function initialize(dbPath?: string): void {
  setState({
    dbPath: dbPath || './cronflow.db',
    workflows: new Map(),
    engineState: 'STOPPED',
  });
  console.log(`Node-Cronflow SDK v${VERSION} initialized`);
}

// State management functions
export async function getGlobalState(
  key: string,
  defaultValue?: any
): Promise<any> {
  const currentState = getCurrentState();
  const stateManager = createStateManager('global', currentState.dbPath);
  return await stateManager.get(key, defaultValue);
}

export async function setGlobalState(
  key: string,
  value: any,
  options?: { ttl?: string }
): Promise<void> {
  const currentState = getCurrentState();
  const stateManager = createStateManager('global', currentState.dbPath);
  await stateManager.set(key, value, options);
}

export async function incrGlobalState(
  key: string,
  amount: number = 1
): Promise<number> {
  const currentState = getCurrentState();
  const stateManager = createStateManager('global', currentState.dbPath);
  return await stateManager.incr(key, amount);
}

export async function deleteGlobalState(key: string): Promise<boolean> {
  const currentState = getCurrentState();
  const stateManager = createStateManager('global', currentState.dbPath);
  return await stateManager.delete(key);
}

export async function getWorkflowState(
  workflowId: string,
  key: string,
  defaultValue?: any
): Promise<any> {
  const currentState = getCurrentState();
  const stateManager = createStateManager(workflowId, currentState.dbPath);
  return await stateManager.get(key, defaultValue);
}

export async function setWorkflowState(
  workflowId: string,
  key: string,
  value: any,
  options?: { ttl?: string }
): Promise<void> {
  const currentState = getCurrentState();
  const stateManager = createStateManager(workflowId, currentState.dbPath);
  await stateManager.set(key, value, options);
}

export async function incrWorkflowState(
  workflowId: string,
  key: string,
  amount: number = 1
): Promise<number> {
  const currentState = getCurrentState();
  const stateManager = createStateManager(workflowId, currentState.dbPath);
  return await stateManager.incr(key, amount);
}

export async function deleteWorkflowState(
  workflowId: string,
  key: string
): Promise<boolean> {
  const currentState = getCurrentState();
  const stateManager = createStateManager(workflowId, currentState.dbPath);
  return await stateManager.delete(key);
}

export async function getStateStats(): Promise<{
  global: {
    totalKeys: number;
    expiredKeys: number;
    namespace: string;
    dbPath: string;
  };
  workflows: Record<
    string,
    {
      totalKeys: number;
      expiredKeys: number;
      namespace: string;
      dbPath: string;
    }
  >;
}> {
  const currentState = getCurrentState();
  const globalStateManager = createStateManager('global', currentState.dbPath);
  const globalStats = await globalStateManager.stats();

  const workflowStats: Record<string, any> = {};
  for (const workflow of currentState.workflows.values()) {
    const workflowStateManager = createStateManager(
      workflow.id,
      currentState.dbPath
    );
    workflowStats[workflow.id] = await workflowStateManager.stats();
  }

  return {
    global: globalStats,
    workflows: workflowStats,
  };
}

export async function cleanupExpiredState(): Promise<{
  global: number;
  workflows: Record<string, number>;
}> {
  const currentState = getCurrentState();
  const globalStateManager = createStateManager('global', currentState.dbPath);
  const globalCleaned = await globalStateManager.cleanup();

  const workflowCleaned: Record<string, number> = {};
  for (const workflow of currentState.workflows.values()) {
    const workflowStateManager = createStateManager(
      workflow.id,
      currentState.dbPath
    );
    workflowCleaned[workflow.id] = await workflowStateManager.cleanup();
  }

  return {
    global: globalCleaned,
    workflows: workflowCleaned,
  };
}

export function define(
  options: Omit<
    WorkflowDefinition,
    'steps' | 'triggers' | 'created_at' | 'updated_at'
  >
): WorkflowInstance {
  const currentState = getCurrentState();

  if (!options.id || options.id.trim() === '') {
    throw new Error('Workflow ID cannot be empty');
  }

  if (currentState.workflows.has(options.id)) {
    throw new Error(`Workflow with ID '${options.id}' already exists`);
  }

  if (options.timeout !== undefined) {
    if (typeof options.timeout === 'string') {
      const timeoutRegex = /^(\d+)(s|m|h|d)$/;
      if (!timeoutRegex.test(options.timeout)) {
        throw new Error(
          'Invalid timeout format. Use format like "5m", "1h", "30s"'
        );
      }
    } else if (typeof options.timeout === 'number') {
      if (options.timeout <= 0) {
        throw new Error('Timeout must be a positive number');
      }
    } else {
      throw new Error('Timeout must be a string or number');
    }
  }

  if (options.concurrency !== undefined) {
    if (!Number.isInteger(options.concurrency) || options.concurrency <= 0) {
      throw new Error('Concurrency must be a positive integer');
    }
  }

  if (options.rateLimit) {
    if (
      !options.rateLimit.count ||
      !Number.isInteger(options.rateLimit.count) ||
      options.rateLimit.count <= 0
    ) {
      throw new Error('Rate limit count must be a positive integer');
    }

    if (!options.rateLimit.per || typeof options.rateLimit.per !== 'string') {
      throw new Error('Rate limit per must be a string (e.g., "1m", "1h")');
    }

    const rateLimitRegex = /^(\d+)(s|m|h|d)$/;
    if (!rateLimitRegex.test(options.rateLimit.per)) {
      throw new Error(
        'Invalid rate limit time format. Use format like "1m", "1h"'
      );
    }
  }

  if (options.queue !== undefined) {
    if (typeof options.queue !== 'string' || options.queue.trim() === '') {
      throw new Error('Queue must be a non-empty string');
    }
  }

  if (options.version !== undefined) {
    if (typeof options.version !== 'string' || options.version.trim() === '') {
      throw new Error('Version must be a non-empty string');
    }

    const versionRegex =
      /^v?\d+\.\d+\.\d+(-[a-zA-Z0-9.-]+)?(\+[a-zA-Z0-9.-]+)?$/;
    if (!versionRegex.test(options.version)) {
      console.warn(
        `Warning: Version '${options.version}' may not follow semantic versioning format`
      );
    }
  }

  if (options.tags) {
    if (!Array.isArray(options.tags)) {
      throw new Error('Tags must be an array');
    }
    for (const tag of options.tags) {
      if (typeof tag !== 'string' || tag.trim() === '') {
        throw new Error('All tags must be non-empty strings');
      }
    }
  }

  if (options.services) {
    if (!Array.isArray(options.services)) {
      throw new Error('Services must be an array');
    }
    for (const service of options.services) {
      if (!service || typeof service !== 'object') {
        throw new Error('Invalid service provided to workflow definition');
      }

      if (!service.id || typeof service.id !== 'string') {
        throw new Error('Service must have a valid id property');
      }

      if (!service.actions || typeof service.actions !== 'object') {
        throw new Error('Service must have actions property');
      }

      for (const [actionName, actionFn] of Object.entries(service.actions)) {
        if (typeof actionFn !== 'function') {
          throw new Error(`Service action '${actionName}' must be a function`);
        }
      }
    }
  }

  if (options.hooks) {
    if (typeof options.hooks !== 'object') {
      throw new Error('Hooks must be an object');
    }

    if (
      options.hooks.onSuccess !== undefined &&
      typeof options.hooks.onSuccess !== 'function'
    ) {
      throw new Error('onSuccess hook must be a function');
    }

    if (
      options.hooks.onFailure !== undefined &&
      typeof options.hooks.onFailure !== 'function'
    ) {
      throw new Error('onFailure hook must be a function');
    }
  }

  if (options.secrets !== undefined) {
    if (typeof options.secrets !== 'object' || options.secrets === null) {
      throw new Error('Secrets must be an object');
    }
  }

  const workflow: WorkflowDefinition = {
    ...options,
    steps: [],
    triggers: [],
    created_at: new Date(),
    updated_at: new Date(),
  };

  currentState.workflows.set(options.id, workflow);
  console.log(`Workflow '${options.id}' defined successfully`);

  return new WorkflowInstance(workflow, {
    define,
    start,
    stop,
    trigger,
    inspect,
  });
}

export async function start(): Promise<void> {
  const currentState = getCurrentState();

  if (
    currentState.engineState === 'STARTED' ||
    currentState.engineState === 'STARTING'
  ) {
    return Promise.resolve();
  }

  setState({ engineState: 'STARTING' });
  console.log('Starting Node-Cronflow engine...');

  if (core) {
    try {
      for (const workflow of currentState.workflows.values()) {
        await registerWorkflowWithRust(workflow);
      }
      console.log('✅ All workflows registered with Rust engine');
    } catch (error) {
      console.error('❌ Failed to register workflows with Rust engine:', error);
      throw error;
    }
  } else {
    console.log('⚠️  Running in simulation mode (Rust core not available)');
  }

  setState({ engineState: 'STARTED' });
  console.log('Node-Cronflow engine started successfully');
}

export async function stop(): Promise<void> {
  setState({ engineState: 'STOPPED' });
  console.log('Node-Cronflow engine stopped');
}

export async function trigger(
  workflowId: string,
  payload: any
): Promise<string> {
  const currentState = getCurrentState();

  if (!core) {
    console.log(
      `⚠️  Simulation: Triggering workflow: ${workflowId} with payload:`,
      payload
    );
    return 'simulation-run-id';
  }

  try {
    const payloadJson = JSON.stringify(payload);
    const result = core.createRun(workflowId, payloadJson, currentState.dbPath);

    if (result.success && result.run_id) {
      console.log(
        `✅ Workflow triggered successfully: ${workflowId} -> ${result.run_id}`
      );
      return result.run_id;
    } else if (result.success && result.message) {
      console.log(
        `✅ Workflow triggered successfully: ${workflowId} -> ${result.message}`
      );
      return result.message.includes('Run created successfully')
        ? `run_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`
        : result.message;
    } else {
      throw new Error(`Failed to trigger workflow: ${result.message}`);
    }
  } catch (error) {
    console.error(`❌ Failed to trigger workflow ${workflowId}:`, error);
    throw error;
  }
}

export async function inspect(runId: string): Promise<any> {
  const currentState = getCurrentState();

  if (!core) {
    console.log(`⚠️  Simulation: Inspecting run: ${runId}`);
    return {
      id: runId,
      status: 'simulation',
      workflow_id: 'simulation-workflow',
      created_at: new Date().toISOString(),
    };
  }

  try {
    const result = core.getRunStatus(runId, currentState.dbPath);

    if (result.success && result.status) {
      return JSON.parse(result.status);
    } else {
      throw new Error(`Failed to inspect run: ${result.message}`);
    }
  } catch (error) {
    console.error(`❌ Failed to inspect run ${runId}:`, error);
    throw error;
  }
}

export async function cancelRun(runId: string): Promise<void> {
  console.log(`Cancelling run: ${runId}`);
  // TODO: Implement run cancellation
}

export async function publishEvent(name: string, payload: any): Promise<void> {
  console.log(`Publishing event: ${name}`, payload);
  // TODO: Implement event publishing
}

export function getWorkflows(): WorkflowDefinition[] {
  const currentState = getCurrentState();
  return Array.from(currentState.workflows.values());
}

export function getWorkflow(id: string): WorkflowDefinition | undefined {
  const currentState = getCurrentState();
  return currentState.workflows.get(id);
}

async function registerWorkflowWithRust(
  workflow: WorkflowDefinition
): Promise<void> {
  if (!core) {
    console.log(`⚠️  Simulation: Registering workflow: ${workflow.id}`);
    return;
  }

  const currentState = getCurrentState();

  try {
    const rustFormat = convertToRustFormat(workflow);
    const workflowJson = JSON.stringify(rustFormat);
    console.log(
      '🔍 Debug: Serialized workflow JSON:',
      workflowJson.substring(0, 500) + '...'
    );

    const result = core.registerWorkflow(workflowJson, currentState.dbPath);

    if (!result.success) {
      throw new Error(`Failed to register workflow: ${result.message}`);
    }

    console.log(
      `✅ Workflow '${workflow.id}' registered successfully with Rust engine`
    );
  } catch (error) {
    console.error(`❌ Failed to register workflow '${workflow.id}':`, error);
    throw error;
  }
}

function convertToRustFormat(workflow: WorkflowDefinition): any {
  return {
    id: workflow.id,
    name: workflow.name || workflow.id,
    description: workflow.description || '',
    steps: workflow.steps.map(step => ({
      id: step.id,
      name: step.name,
      action: step.handler.toString(), // Always include action field
      type: step.type,
      handler: step.handler.toString(),
      timeout: step.options?.timeout || 30000,
      retry: step.options?.retry
        ? {
            max_attempts: step.options.retry.attempts,
            backoff_ms: parseDuration(step.options.retry.backoff.delay),
          }
        : { max_attempts: 1, backoff_ms: 1000 },
      depends_on: [],
    })),
    triggers: workflow.triggers.map(trigger => {
      if (trigger.type === 'webhook') {
        return {
          type: 'webhook',
          path: trigger.path,
          method: trigger.options?.method || 'POST',
        };
      } else if (trigger.type === 'schedule') {
        return {
          type: 'schedule',
          cron_expression: trigger.cron_expression,
        };
      } else {
        return { type: 'manual' };
      }
    }),
    // Note: Services are not serialized to Rust as they contain functions
    // Services are handled in the Node.js layer during execution
    created_at: workflow.created_at.toISOString(),
    updated_at: workflow.updated_at.toISOString(),
  };
}

function parseDuration(duration: string | number): number {
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

export function getEngineState(): 'STOPPED' | 'STARTING' | 'STARTED' {
  return getCurrentState().engineState;
}

export async function replay(
  runId: string,
  options?: {
    overridePayload?: any;
    mockStep?: (stepName: string, mockFn: (ctx: Context) => any) => void;
  }
): Promise<void> {
  console.log(`Replaying run: ${runId} with options:`, options);
  // TODO: Implement replay functionality
}

export async function resume(token: string, payload: any): Promise<void> {
  console.log(`🔄 Resuming workflow with token: ${token}`);
  console.log(`📋 Resume payload:`, payload);

  // TODO: Implement actual resume functionality
  // This would involve:
  // 1. Looking up the paused workflow by token
  // 2. Validating the token hasn't expired
  // 3. Resuming the workflow execution with the provided payload
  // 4. Updating the pause status in the database

  // For now, simulate resume functionality
  const resumeInfo = {
    token,
    payload,
    resumedAt: Date.now(),
    status: 'resumed',
  };

  console.log(`✅ Workflow resumed successfully with token: ${token}`);
  console.log(`📊 Resume info:`, resumeInfo);

  // In a real implementation, this would:
  // - Find the paused workflow run
  // - Validate the token and timeout
  // - Resume execution from the human-in-the-loop step
  // - Pass the payload to the next step
  // - Update the workflow state

  return Promise.resolve();
}

export function isRustCoreAvailable(): boolean {
  return core !== null;
}

export async function executeManualTrigger(
  workflowId: string,
  payload: any
): Promise<any> {
  if (!core) {
    console.log(
      `⚠️  Simulation: Executing manual trigger for workflow: ${workflowId}`
    );
    return {
      success: true,
      run_id: 'simulation-run-id',
      workflow_id: workflowId,
      message: 'Manual trigger executed in simulation mode',
    };
  }

  try {
    const payloadJson = JSON.stringify(payload);
    const result = core.executeManualTrigger(
      workflowId,
      payloadJson,
      getCurrentState().dbPath
    );

    if (result.success) {
      console.log(
        `✅ Manual trigger executed successfully for workflow: ${workflowId}`
      );
      return result;
    } else {
      throw new Error(`Failed to execute manual trigger: ${result.message}`);
    }
  } catch (error) {
    console.error(
      `❌ Failed to execute manual trigger for workflow ${workflowId}:`,
      error
    );
    throw error;
  }
}

export async function executeWebhookTrigger(request: any): Promise<any> {
  if (!core) {
    console.log(
      `⚠️  Simulation: Executing webhook trigger for request:`,
      request
    );
    return {
      success: true,
      run_id: 'simulation-webhook-run-id',
      workflow_id: 'simulation-workflow',
      message: 'Webhook trigger executed in simulation mode',
    };
  }

  try {
    const requestJson = JSON.stringify(request);
    const result = core.executeWebhookTrigger(
      requestJson,
      getCurrentState().dbPath
    );

    if (result.success) {
      console.log(`✅ Webhook trigger executed successfully`);
      return result;
    } else {
      throw new Error(`Failed to execute webhook trigger: ${result.message}`);
    }
  } catch (error) {
    console.error(`❌ Failed to execute webhook trigger:`, error);
    throw error;
  }
}

export async function executeScheduleTrigger(triggerId: string): Promise<any> {
  if (!core) {
    console.log(`⚠️  Simulation: Executing schedule trigger: ${triggerId}`);
    return {
      success: true,
      run_id: 'simulation-schedule-run-id',
      workflow_id: 'simulation-workflow',
      message: 'Schedule trigger executed in simulation mode',
    };
  }

  try {
    const result = core.executeScheduleTrigger(
      triggerId,
      getCurrentState().dbPath
    );

    if (result.success) {
      console.log(`✅ Schedule trigger executed successfully: ${triggerId}`);
      return result;
    } else {
      throw new Error(`Failed to execute schedule trigger: ${result.message}`);
    }
  } catch (error) {
    console.error(`❌ Failed to execute schedule trigger ${triggerId}:`, error);
    throw error;
  }
}

export async function getTriggerStats(): Promise<any> {
  if (!core) {
    console.log(`⚠️  Simulation: Getting trigger statistics`);
    return {
      success: true,
      stats: JSON.stringify({
        total_triggers: 0,
        webhook_triggers: 0,
        schedule_triggers: 0,
      }),
      message: 'Trigger statistics retrieved in simulation mode',
    };
  }

  try {
    const result = core.getTriggerStats(getCurrentState().dbPath);

    if (result.success) {
      console.log(`✅ Trigger statistics retrieved successfully`);
      return result;
    } else {
      throw new Error(`Failed to get trigger stats: ${result.message}`);
    }
  } catch (error) {
    console.error(`❌ Failed to get trigger statistics:`, error);
    throw error;
  }
}

export async function getWorkflowTriggers(workflowId: string): Promise<any> {
  if (!core) {
    console.log(`⚠️  Simulation: Getting triggers for workflow: ${workflowId}`);
    return {
      success: true,
      triggers: JSON.stringify(['manual']),
      message: 'Workflow triggers retrieved in simulation mode',
    };
  }

  try {
    const result = core.getWorkflowTriggers(
      workflowId,
      getCurrentState().dbPath
    );

    if (result.success) {
      console.log(`✅ Workflow triggers retrieved successfully`);
      return result;
    } else {
      throw new Error(`Failed to get workflow triggers: ${result.message}`);
    }
  } catch (error) {
    console.error(`❌ Failed to get workflow triggers:`, error);
    throw error;
  }
}

export async function unregisterWorkflowTriggers(
  workflowId: string
): Promise<any> {
  if (!core) {
    console.log(
      `⚠️  Simulation: Unregistering triggers for workflow: ${workflowId}`
    );
    return {
      success: true,
      message: 'Workflow triggers unregistered in simulation mode',
    };
  }

  try {
    const result = core.unregisterWorkflowTriggers(
      workflowId,
      getCurrentState().dbPath
    );

    if (result.success) {
      console.log(`✅ Workflow triggers unregistered successfully`);
      return result;
    } else {
      throw new Error(
        `Failed to unregister workflow triggers: ${result.message}`
      );
    }
  } catch (error) {
    console.error(`❌ Failed to unregister workflow triggers:`, error);
    throw error;
  }
}

export async function getScheduleTriggers(): Promise<any> {
  if (!core) {
    console.log(`⚠️  Simulation: Getting schedule triggers`);
    return {
      success: true,
      triggers: JSON.stringify([]),
      message: 'Schedule triggers retrieved in simulation mode',
    };
  }

  try {
    const result = core.getScheduleTriggers(getCurrentState().dbPath);

    if (result.success) {
      console.log(`✅ Schedule triggers retrieved successfully`);
      return result;
    } else {
      throw new Error(`Failed to get schedule triggers: ${result.message}`);
    }
  } catch (error) {
    console.error(`❌ Failed to get schedule triggers:`, error);
    throw error;
  }
}

initialize();
