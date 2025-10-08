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
import { concurrencyManager } from './execution/concurrency-manager';
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
import {
  executeManualTrigger as executeManualTriggerFromModule,
  executeWebhookTrigger as executeWebhookTriggerFromModule,
  executeScheduleTrigger as executeScheduleTriggerFromModule,
  getTriggerStats as getTriggerStatsFromModule,
  getWorkflowTriggers as getWorkflowTriggersFromModule,
  unregisterWorkflowTriggers as unregisterWorkflowTriggersFromModule,
  getScheduleTriggers as getScheduleTriggersFromModule,
  setTriggerManagerState,
} from './triggers';
import {
  registerWorkflowWithRust as registerWorkflowWithRustFromModule,
  convertToRustFormat as convertToRustFormatFromModule,
  parseDuration as parseDurationFromModule,
  isRustCoreAvailable as isRustCoreAvailableFromModule,
  getCoreStatus as getCoreStatusFromModule,
  setRustIntegrationState,
} from './rust';

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
  setWorkflowEngineState(state);
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
    dbPath: dbPath || './.cronflow/data.db',
    workflows: new Map(),
    engineState: 'STOPPED',
    eventListeners: new Map(),
    eventHistory: [],
  });

  setWorkflowEngineState(getCurrentState());
  setStepRegistry(stepRegistry);
  setEventSystemState(
    getCurrentState().eventListeners,
    getCurrentState().eventHistory
  );
  setGetWorkflowFunction(getWorkflow);
  setBenchmarkDependencies(define, start, stop, trigger);
  setTriggerManagerState(getCurrentState);
  setRustIntegrationState(getCurrentState);
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

  if (!scheduler.getRunningStatus()) {
    scheduler.start();
  }

  if (!currentState.dbPath) {
    const defaultDbPath = './.cronflow/data.db';
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
        await registerWorkflowWithRustFromModule(workflow);
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
  }

  setState({ engineState: 'STARTED' });
}

export async function stop(): Promise<void> {
  const currentState = getCurrentState();

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

    const runId =
      result.success && result.runId
        ? result.runId
        : `run_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;

    if (result.success || !result.runId) {
      // Execute with concurrency control
      await concurrencyManager.executeWithConcurrency(
        workflowId,
        runId,
        payload,
        () => executeWorkflowSteps(workflowId, runId, payload)
      );
      return runId;
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

export function getWorkflows(): WorkflowDefinition[] {
  const currentState = getCurrentState();
  return Array.from(currentState.workflows.values());
}

export function getWorkflow(id: string): WorkflowDefinition | undefined {
  const currentState = getCurrentState();
  return currentState.workflows.get(id);
}

export function getConcurrencyStats(workflowId?: string): any {
  if (workflowId) {
    return concurrencyManager.getStats(workflowId);
  } else {
    return concurrencyManager.getAllStats();
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

export async function publishEvent(name: string, payload: any): Promise<void> {
  return await publishEventFromModule(name, payload);
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

export function isRustCoreAvailable(): boolean {
  return isRustCoreAvailableFromModule();
}

export function getCoreStatus(): {
  available: boolean;
  path?: string;
  error?: string;
} {
  return getCoreStatusFromModule();
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
  return await executeManualTriggerFromModule(workflowId, payload);
}

export async function executeWebhookTrigger(request: any): Promise<any> {
  return await executeWebhookTriggerFromModule(request);
}

export async function executeScheduleTrigger(triggerId: string): Promise<any> {
  return await executeScheduleTriggerFromModule(triggerId);
}

export async function getTriggerStats(): Promise<any> {
  return await getTriggerStatsFromModule();
}

export async function getWorkflowTriggers(workflowId: string): Promise<any> {
  return await getWorkflowTriggersFromModule(workflowId);
}

export async function unregisterWorkflowTriggers(
  workflowId: string
): Promise<any> {
  return await unregisterWorkflowTriggersFromModule(workflowId);
}

export async function getScheduleTriggers(): Promise<any> {
  return await getScheduleTriggersFromModule();
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

export function registerEventListener(
  eventName: string,
  workflowId: string,
  trigger: any
): void {
  registerEventListenerFromModule(eventName, workflowId, trigger);
  const eventState = getEventListenersState();
  setState({
    eventListeners: eventState.listeners,
    eventHistory: eventState.history,
  });
}

export function unregisterEventListener(
  eventName: string,
  workflowId: string
): void {
  unregisterEventListenerFromModule(eventName, workflowId);
  const eventState = getEventListenersState();
  setState({
    eventListeners: eventState.listeners,
    eventHistory: eventState.history,
  });
}

export function getEventHistory(
  eventName?: string
): Array<{ name: string; payload: any; timestamp: number }> {
  return getEventHistoryFromModule(eventName);
}

export function getEventListeners(
  eventName?: string
): Map<string, Array<{ workflowId: string; trigger: any }>> {
  return getEventListenersFromModule(eventName);
}

initialize();

registerHookHandlerFromModule('onSuccess', executeWorkflowHook);
registerHookHandlerFromModule('onFailure', executeWorkflowHook);

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
