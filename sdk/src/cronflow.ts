import { WorkflowInstance, WorkflowDefinition, Context } from './workflow';
import * as http from 'http';
import { scheduler } from './scheduler';
import { loadCoreModule } from './utils/core-resolver';
import {
  executeWorkflowSteps,
  executeStep,
  executeStepFunction,
  executeJobFunction,
  createValidContext,
  setWorkflowEngineState,
  setStepRegistry,
} from './execution/workflow-engine';
import { createWebhookServer } from './webhook';
import {
  getGlobalState as getGlobalStateFromModule,
  setGlobalState as setGlobalStateFromModule,
  incrGlobalState as incrGlobalStateFromModule,
  deleteGlobalState as deleteGlobalStateFromModule,
  getStateStats as getStateStatsFromModule,
  cleanupExpiredState as cleanupExpiredStateFromModule,
} from './state/global-state';
import {
  getWorkflowState as getWorkflowStateFromModule,
  setWorkflowState as setWorkflowStateFromModule,
  incrWorkflowState as incrWorkflowStateFromModule,
  deleteWorkflowState as deleteWorkflowStateFromModule,
} from './state/workflow-state';
import {
  resume as resumeFromModule,
  storePausedWorkflow as storePausedWorkflowFromModule,
  getPausedWorkflow as getPausedWorkflowFromModule,
  listPausedWorkflows as listPausedWorkflowsFromModule,
} from './human-loop';
import {
  publishEvent as publishEventFromModule,
  registerEventListener as registerEventListenerFromModule,
  unregisterEventListener as unregisterEventListenerFromModule,
  getEventHistory as getEventHistoryFromModule,
  getEventListeners as getEventListenersFromModule,
  setEventSystemState,
  getEventListenersState,
} from './events';
import {
  registerHookHandler as registerHookHandlerFromModule,
  getHookHandler as getHookHandlerFromModule,
  executeWorkflowHook as executeWorkflowHookFromModule,
  setGetWorkflowFunction,
} from './hooks';
import {
  benchmark as benchmarkFromModule,
  setBenchmarkDependencies,
  type BenchmarkOptions,
  type BenchmarkResult,
} from './performance/benchmark';

// Import the Rust addon
const { core } = loadCoreModule();

export const VERSION = '0.1.0';

interface CronflowState {
  workflows: Map<string, WorkflowDefinition>;
  engineState: 'STOPPED' | 'STARTING' | 'STARTED';
  dbPath: string;
  stateManager?: any;
  webhookServer?: http.Server;
  eventListeners: Map<string, Array<{ workflowId: string; trigger: any }>>;
  eventHistory: Array<{ name: string; payload: any; timestamp: number }>;
}

interface StepHandler {
  workflowId: string;
  stepId: string;
  handler: (ctx: Context) => any | Promise<any>;
  type: 'step' | 'action';
}

let stepRegistry: Map<string, StepHandler> = new Map();

let state: CronflowState = {
  workflows: new Map(),
  engineState: 'STOPPED',
  dbPath: './cronflow.db',
  eventListeners: new Map(),
  eventHistory: [],
};

function getCurrentState(): CronflowState {
  return state;
}

function setState(newState: Partial<CronflowState>): void {
  state = { ...state, ...newState };
  // Keep workflow engine state in sync
  setWorkflowEngineState(state);
  // Keep event system state in sync
  setEventSystemState(state.eventListeners, state.eventHistory);
}

function registerStepHandler(
  workflowId: string,
  stepId: string,
  handler: (ctx: Context) => any | Promise<any>,
  type: 'step' | 'action'
): void {
  const key = `${workflowId}:${stepId}`;
  stepRegistry.set(key, {
    workflowId,
    stepId,
    handler,
    type,
  });
  // Keep workflow engine step registry in sync
  setStepRegistry(stepRegistry);
}

function getStepHandler(
  workflowId: string,
  stepId: string
): StepHandler | undefined {
  const key = `${workflowId}:${stepId}`;
  return stepRegistry.get(key);
}

function initialize(dbPath?: string): void {
  setState({
    dbPath: dbPath || './cronflow.db',
    workflows: new Map(),
    engineState: 'STOPPED',
    eventListeners: new Map(),
    eventHistory: [],
  });

  // Initialize workflow engine with current state
  setWorkflowEngineState(getCurrentState());
  setStepRegistry(stepRegistry);

  // Initialize event system state
  setEventSystemState(
    getCurrentState().eventListeners,
    getCurrentState().eventHistory
  );

  // Initialize hook system state
  setGetWorkflowFunction(getWorkflow);

  // Initialize benchmark dependencies
  setBenchmarkDependencies(define, start, stop, trigger);
}

export async function getGlobalState(
  key: string,
  defaultValue?: any
): Promise<any> {
  return await getGlobalStateFromModule(key, getCurrentState, defaultValue);
}

export async function setGlobalState(
  key: string,
  value: any,
  options?: { ttl?: string }
): Promise<void> {
  return await setGlobalStateFromModule(key, value, getCurrentState, options);
}

export async function incrGlobalState(
  key: string,
  amount: number = 1
): Promise<number> {
  return await incrGlobalStateFromModule(key, getCurrentState, amount);
}

export async function deleteGlobalState(key: string): Promise<boolean> {
  return await deleteGlobalStateFromModule(key, getCurrentState);
}

export async function getWorkflowState(
  workflowId: string,
  key: string,
  defaultValue?: any
): Promise<any> {
  return await getWorkflowStateFromModule(
    workflowId,
    key,
    getCurrentState,
    defaultValue
  );
}

export async function setWorkflowState(
  workflowId: string,
  key: string,
  value: any,
  options?: { ttl?: string }
): Promise<void> {
  return await setWorkflowStateFromModule(
    workflowId,
    key,
    value,
    getCurrentState,
    options
  );
}

export async function incrWorkflowState(
  workflowId: string,
  key: string,
  amount: number = 1
): Promise<number> {
  return await incrWorkflowStateFromModule(
    workflowId,
    key,
    getCurrentState,
    amount
  );
}

export async function deleteWorkflowState(
  workflowId: string,
  key: string
): Promise<boolean> {
  return await deleteWorkflowStateFromModule(workflowId, key, getCurrentState);
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
  return await getStateStatsFromModule(getCurrentState);
}

export async function cleanupExpiredState(): Promise<{
  global: number;
  workflows: Record<string, number>;
}> {
  return await cleanupExpiredStateFromModule(getCurrentState);
}

export function define(
  options: Omit<
    WorkflowDefinition,
    'steps' | 'triggers' | 'created_at' | 'updated_at'
  >
): WorkflowInstance {
  if (!options.id || options.id.trim() === '') {
    throw new Error('Workflow ID cannot be empty');
  }

  const currentState = getCurrentState();
  if (currentState.workflows.has(options.id)) {
    throw new Error(`Workflow with ID '${options.id}' already exists`);
  }

  const workflow: WorkflowDefinition = {
    ...options,
    steps: [],
    triggers: [],
    created_at: new Date(),
    updated_at: new Date(),
  };

  const instance = new WorkflowInstance(workflow, cronflow);

  // Register the workflow with the SDK state
  currentState.workflows.set(workflow.id, workflow);

  return instance;
}

export interface StartOptions {
  webhookServer?: {
    host?: string;
    port?: number;
    maxConnections?: number;
  };
}

export async function start(options?: StartOptions): Promise<void> {
  const currentState = getCurrentState();

  if (
    currentState.engineState === 'STARTED' ||
    currentState.engineState === 'STARTING'
  ) {
    return Promise.resolve();
  }

  setState({ engineState: 'STARTING' });

  // Start the Node.js scheduler (only if not already running)
  if (!scheduler.getRunningStatus()) {
    scheduler.start();
  } else {
  }

  if (!currentState.dbPath) {
    const defaultDbPath = './cronflow.db';
    setState({ dbPath: defaultDbPath });
  }

  if (core) {
    try {
      if (options?.webhookServer) {
        const webhookConfig = {
          host: options.webhookServer.host || '127.0.0.1',
          port: options.webhookServer.port || 3000,
          max_connections: options.webhookServer.maxConnections || 1000,
        };

        // TODO: Expose webhook server configuration through N-API
        // For now, we'll use environment variables as a workaround
        if (webhookConfig.host !== '127.0.0.1') {
          process.env.CRONFLOW_WEBHOOK_HOST = webhookConfig.host;
        }
        if (webhookConfig.port !== 3000) {
          process.env.CRONFLOW_WEBHOOK_PORT = webhookConfig.port.toString();
        }
        if (webhookConfig.max_connections !== 1000) {
          process.env.CRONFLOW_WEBHOOK_MAX_CONNECTIONS =
            webhookConfig.max_connections.toString();
        }
      }

      for (const workflow of currentState.workflows.values()) {
        await registerWorkflowWithRust(workflow);
      }

      if (options?.webhookServer) {
        const webhookServer = createWebhookServer(
          options.webhookServer,
          getCurrentState,
          setState,
          trigger
        );
        setState({ webhookServer: webhookServer.server });
      }
    } catch (error) {
      throw error;
    }
  } else {
    // Running in simulation mode
  }

  setState({ engineState: 'STARTED' });
}

export async function stop(): Promise<void> {
  const currentState = getCurrentState();

  // Stop the Node.js scheduler
  scheduler.stop();

  if (currentState.webhookServer) {
    currentState.webhookServer.close(() => {
      // Webhook server closed
    });

    setState({ webhookServer: undefined });
  }

  setState({ engineState: 'STOPPED' });
}

export async function trigger(
  workflowId: string,
  payload: any
): Promise<string> {
  const currentState = getCurrentState();

  if (!core) {
    return 'simulation-run-id';
  }

  try {
    const payloadJson = JSON.stringify(payload);
    const result = core.createRun(workflowId, payloadJson, currentState.dbPath);

    if (result.success && result.runId) {
      await executeWorkflowSteps(workflowId, result.runId, payload);

      return result.runId;
    } else if (result.success) {
      // Fallback to generating a run ID if none was returned
      const fallbackRunId = `run_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;

      await executeWorkflowSteps(workflowId, fallbackRunId, payload);

      return fallbackRunId;
    } else {
      throw new Error(`Failed to trigger workflow: ${result.message}`);
    }
  } catch (error) {
    throw error;
  }
}

export async function inspect(runId: string): Promise<any> {
  const currentState = getCurrentState();

  if (!core) {
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
    throw error;
  }
}

export async function cancelRun(runId: string): Promise<void> {
  // TODO: Implement run cancellation
}

export async function publishEvent(name: string, payload: any): Promise<void> {
  return await publishEventFromModule(name, payload);
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
    return;
  }

  const currentState = getCurrentState();

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

function convertToRustFormat(workflow: WorkflowDefinition): any {
  return {
    id: workflow.id,
    name: workflow.name || workflow.id,
    description: workflow.description || '',
    steps: workflow.steps.map(step => ({
      id: step.id,
      name: step.name,
      title: step.title,
      description: step.description,
      action: step.handler.toString(),
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
  // TODO: Implement replay functionality
}

export async function resume(token: string, payload: any): Promise<void> {
  return await resumeFromModule(token, payload);
}

export function storePausedWorkflow(
  token: string,
  pauseInfo: {
    token: string;
    workflowId: string;
    runId: string;
    stepId: string;
    description: string;
    metadata?: Record<string, any>;
    createdAt: number;
    expiresAt?: number;
    status: 'waiting' | 'resumed' | 'timeout';
    payload: any;
    lastStepOutput: any;
    resumeCallback?: (payload: any) => void;
  }
): void {
  return storePausedWorkflowFromModule(token, pauseInfo);
}

export function getPausedWorkflow(token: string) {
  return getPausedWorkflowFromModule(token);
}

export function listPausedWorkflows() {
  return listPausedWorkflowsFromModule();
}

// Function to check if Rust core is available
export function isRustCoreAvailable(): boolean {
  return core !== null;
}

// Function to get core loading status
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

export async function benchmark(
  options: BenchmarkOptions = {}
): Promise<BenchmarkResult> {
  return await benchmarkFromModule(options);
}

export async function executeManualTrigger(
  workflowId: string,
  payload: any
): Promise<any> {
  if (!core) {
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
      return result;
    } else {
      throw new Error(`Failed to execute manual trigger: ${result.message}`);
    }
  } catch (error) {
    throw error;
  }
}

export async function executeWebhookTrigger(request: any): Promise<any> {
  if (!core) {
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
      return result;
    } else {
      throw new Error(`Failed to execute webhook trigger: ${result.message}`);
    }
  } catch (error) {
    throw error;
  }
}

export async function executeScheduleTrigger(triggerId: string): Promise<any> {
  if (!core) {
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
      return result;
    } else {
      throw new Error(`Failed to execute schedule trigger: ${result.message}`);
    }
  } catch (error) {
    throw error;
  }
}

export async function getTriggerStats(): Promise<any> {
  if (!core) {
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
      return result;
    } else {
      throw new Error(`Failed to get trigger stats: ${result.message}`);
    }
  } catch (error) {
    throw error;
  }
}

export async function getWorkflowTriggers(workflowId: string): Promise<any> {
  if (!core) {
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
      return result;
    } else {
      throw new Error(`Failed to get workflow triggers: ${result.message}`);
    }
  } catch (error) {
    throw error;
  }
}

export async function unregisterWorkflowTriggers(
  workflowId: string
): Promise<any> {
  if (!core) {
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
      return result;
    } else {
      throw new Error(
        `Failed to unregister workflow triggers: ${result.message}`
      );
    }
  } catch (error) {
    throw error;
  }
}

export async function getScheduleTriggers(): Promise<any> {
  if (!core) {
    return {
      success: true,
      triggers: JSON.stringify([]),
      message: 'Schedule triggers retrieved in simulation mode',
    };
  }

  try {
    const result = core.getScheduleTriggers(getCurrentState().dbPath);

    if (result.success) {
      return result;
    } else {
      throw new Error(`Failed to get schedule triggers: ${result.message}`);
    }
  } catch (error) {
    throw error;
  }
}

initialize();

registerHookHandler('onSuccess', executeWorkflowHook);
registerHookHandler('onFailure', executeWorkflowHook);

function registerHookHandler(
  hookType: 'onSuccess' | 'onFailure',
  handler: (
    hookType: string,
    contextJson: string,
    workflowId: string,
    stepId?: string
  ) => Promise<any>
): void {
  registerHookHandlerFromModule(hookType, handler);
}

function getHookHandler(
  hookType: string
):
  | ((
      hookType: string,
      contextJson: string,
      workflowId: string,
      stepId?: string
    ) => Promise<any>)
  | undefined {
  return getHookHandlerFromModule(hookType);
}

export async function executeWorkflowHook(
  hookType: string,
  contextJson: string,
  workflowId: string,
  stepId?: string
): Promise<any> {
  return await executeWorkflowHookFromModule(
    hookType,
    contextJson,
    workflowId,
    stepId
  );
}

function registerEventListener(
  eventName: string,
  workflowId: string,
  trigger: any
): void {
  registerEventListenerFromModule(eventName, workflowId, trigger);
  // Update the main state to keep it in sync
  const eventState = getEventListenersState();
  setState({
    eventListeners: eventState.listeners,
    eventHistory: eventState.history,
  });
}

function unregisterEventListener(eventName: string, workflowId: string): void {
  unregisterEventListenerFromModule(eventName, workflowId);
  // Update the main state to keep it in sync
  const eventState = getEventListenersState();
  setState({
    eventListeners: eventState.listeners,
    eventHistory: eventState.history,
  });
}

function getEventHistory(
  eventName?: string
): Array<{ name: string; payload: any; timestamp: number }> {
  return getEventHistoryFromModule(eventName);
}

function getEventListeners(
  eventName?: string
): Map<string, Array<{ workflowId: string; trigger: any }>> {
  return getEventListenersFromModule(eventName);
}

export {
  registerEventListener,
  unregisterEventListener,
  getEventHistory,
  getEventListeners,
};

export const cronflow = {
  define,
  start,
  stop,
  trigger,
  inspect,
  cancelRun,
  publishEvent,
  executeStep,
  executeStepFunction,
  executeJobFunction,
  getWorkflows,
  getWorkflow,
  getEngineState,
  replay,
  resume,
  isRustCoreAvailable,
  benchmark,
  executeManualTrigger,
  executeWebhookTrigger,
  executeScheduleTrigger,
  getTriggerStats,
  getWorkflowTriggers,
  unregisterWorkflowTriggers,
  getScheduleTriggers,
  createValidContext,
  executeWorkflowHook,
  getGlobalState,
  setGlobalState,
  incrGlobalState,
  deleteGlobalState,
  getWorkflowState,
  setWorkflowState,
  incrWorkflowState,
  deleteWorkflowState,
  getStateStats,
  cleanupExpiredState,
  registerStepHandler,
  VERSION,
  registerEventListener,
  unregisterEventListener,
  getEventHistory,
  getEventListeners,
  scheduler,
};
