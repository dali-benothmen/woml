#!/usr/bin/env node

/**
 * Generate TypeScript declaration files for the built packages
 * This is needed because Bun build doesn't generate .d.ts files
 */

const fs = require('fs');
const path = require('path');

console.log('📝 Generating TypeScript declaration files...');

// Create comprehensive declaration files based on actual source code
const declarations = {
  'dist/index.d.ts': `/**
 * Node-Cronflow Main Package
 */

export * from './sdk/index';

// Explicitly export key types for better IDE support
export type {
  Context,
  WorkflowDefinition,
  StepDefinition,
  TriggerDefinition,
  RetryConfig,
  RetryBackoffConfig,
  StepOptions,
  WebhookOptions,
  CacheConfig,
  WorkflowInstance,
  PausedWorkflow,
  EventListener,
  EventHistoryItem,
  BenchmarkOptions,
  BenchmarkResult,
} from './sdk/index';

export { cronflow as default } from './sdk/index';
`,

  'dist/sdk/index.d.ts': `/**
 * Node-Cronflow SDK
 */
import { z } from 'zod';

export const VERSION: string;

// ============================================================================
// Core Interfaces
// ============================================================================

export interface Context {
  payload: any;
  steps: Record<string, { output: any }>;
  run: {
    id: string;
    workflowId: string;
  };
  state: {
    get: (key: string, defaultValue?: any) => any;
    set: (key: string, value: any, options?: { ttl?: string }) => Promise<void>;
    incr: (key: string, amount?: number) => Promise<number>;
  };
  last: any;
  trigger: {
    headers: Record<string, string>;
    rawBody?: Buffer;
  };
  cancel: (reason?: string) => never;
  error?: Error;
  step_name?: string;
  step_result?: any;
  step_status?: 'completed' | 'failed';
  step_error?: string;
  background?: boolean;
  token?: string | null;
}

export interface CacheConfig {
  key: (ctx: Context) => string;
  ttl: string;
}

export interface RetryBackoffConfig {
  strategy?: 'fixed' | 'linear' | 'exponential';
  delay: string | number;
  maxDelay?: string | number;
  multiplier?: number;
}

export interface RetryConfig {
  attempts: number;
  backoff?: RetryBackoffConfig;
  retryOn?: {
    errors?: string[];
    statusCodes?: number[];
    conditions?: (error: Error, attempt: number) => boolean;
  };
  onRetry?: (
    error: Error,
    attempt: number,
    nextDelay: number
  ) => void | Promise<void>;
}

export interface StepOptions {
  timeout?: string | number;
  retry?: RetryConfig;
  circuitBreaker?: {
    name?: string;
    failureThreshold?: number;
    recoveryTimeout?: string | number;
    expectedErrors?: string[];
  };
  cache?: CacheConfig;
  delay?: string | number;
  parallel?: boolean;
  race?: boolean;
  loop?: boolean;
  maxIterations?: number;
  controlFlow?: boolean;
  endIf?: boolean;
  conditional?: boolean;
  conditionType?: 'if' | 'elseIf' | 'else';
  stepCount?: number;
  parallelGroupId?: string;
  parallelStepCount?: number;
  cancel?: boolean;
  reason?: string;
  subflow?: boolean;
  workflowId?: string;
  input?: any;
  forEach?: boolean;
  batch?: boolean;
  batchSize?: number;
  humanInTheLoop?: boolean;
  token?: string;
  description?: string;
  approvalUrl?: string;
  metadata?: Record<string, any>;
  createdAt?: number;
  expiresAt?: number;
  waitForEvent?: boolean;
  eventName?: string;
  onError?: (ctx: Context) => any;
  background?: boolean;
  pause?: boolean;
}

export interface StepDefinition {
  id: string;
  name: string;
  title?: string;
  description?: string;
  handler: (ctx: Context) => any | Promise<any>;
  type: 'step' | 'action';
  options?: StepOptions;
  parallel?: boolean;
  parallel_group_id?: string;
  parallel_step_count?: number;
  race?: boolean;
  for_each?: boolean;
}

export interface WebhookOptions {
  method?: 'POST' | 'GET' | 'PUT' | 'DELETE';
  schema?: z.ZodObject<any>;
  validate?: (payload: any) => boolean | string | Promise<boolean | string>;
  idempotencyKey?: (ctx: Context) => string;
  parseRawBody?: boolean;
  app?: any;
  appInstance?: any;
  registerRoute?: (method: string, path: string, handler: Function) => void;
  headers?: {
    required?: Record<string, string>;
    validate?: (headers: Record<string, string>) => boolean | string;
  };
  trigger?: string;
  condition?: (req: any) => boolean | Promise<boolean>;
  middleware?: Array<
    (req: any, res: any, next: () => void) => void | Promise<void>
  >;
  onSuccess?: (ctx: Context, result?: any) => void | Promise<void>;
  onError?: (ctx: Context, error: Error) => void | Promise<void>;
  retry?: RetryConfig;
}

export type TriggerDefinition =
  | { type: 'webhook'; path: string; options?: WebhookOptions }
  | { type: 'schedule'; cron_expression: string }
  | { type: 'event'; eventName: string }
  | { type: 'manual' };

export interface WorkflowDefinition {
  id: string;
  name?: string;
  description?: string;
  tags?: string[];
  hooks?: {
    onSuccess?: (ctx: Context, stepId?: string | string[]) => void;
    onFailure?: (ctx: Context, stepId?: string | string[]) => void;
  };
  timeout?: string | number;
  concurrency?: number;
  rateLimit?: {
    count: number;
    per: string;
  };
  queue?: string;
  version?: string;
  secrets?: object;
  steps: StepDefinition[];
  triggers: TriggerDefinition[];
  created_at: Date;
  updated_at: Date;
}

// ============================================================================
// Classes
// ============================================================================

export interface StepConfig {
  id: string;
  title?: string;
  description?: string;
}

export declare class WorkflowInstance {
  constructor(workflow: WorkflowDefinition, cronflowInstance: any);
  
  // Core step methods
  step(nameOrConfig: string | StepConfig, handlerFn: (ctx: Context) => any | Promise<any>, options?: StepOptions): WorkflowInstance;
  action(nameOrConfig: string | StepConfig, handlerFn: (ctx: Context) => any | Promise<any>, options?: StepOptions): WorkflowInstance;
  
  // Step modifiers (chainable after step/action)
  retry(options: RetryConfig): WorkflowInstance;
  timeout(duration: string | number): WorkflowInstance;
  cache(config: CacheConfig): WorkflowInstance;
  delay(duration: string | number): WorkflowInstance;
  onError(handlerFn: (ctx: Context) => any): WorkflowInstance;
  
  // Triggers
  onWebhook(path: string, options?: WebhookOptions): WorkflowInstance;
  onSchedule(cronExpression: string): WorkflowInstance;
  onInterval(interval: string): WorkflowInstance;
  onEvent(eventName: string): WorkflowInstance;
  manual(): WorkflowInstance;
  
  // Control flow
  if(name: string, condition: (ctx: Context) => boolean | Promise<boolean>, options?: any): WorkflowInstance;
  elseIf(name: string, condition: (ctx: Context) => boolean | Promise<boolean>): WorkflowInstance;
  else(): WorkflowInstance;
  endIf(): WorkflowInstance;
  
  // Parallel execution
  parallel(steps: Array<(ctx: Context) => any | Promise<any>>): WorkflowInstance;
  race(steps: Array<(ctx: Context) => any | Promise<any>>): WorkflowInstance;
  
  // Loops and iteration
  forEach(name: string, items: (ctx: Context) => any[] | Promise<any[]>, iterationFn: (item: any, flow: WorkflowInstance) => void): WorkflowInstance;
  while(name: string, condition: (ctx: Context) => boolean | Promise<boolean>, iterationFn: (ctx: Context) => void): WorkflowInstance;
  batch(name: string, options: { items: (ctx: Context) => any[] | Promise<any[]>; size: number }, batchFn: (batch: any[], flow: WorkflowInstance) => void): WorkflowInstance;
  
  // Advanced features
  sleep(duration: string | number): WorkflowInstance;
  subflow(name: string, workflowId: string, input?: any): WorkflowInstance;
  cancel(reason?: string): WorkflowInstance;
  humanInTheLoop(options: { timeout?: string; onPause: (ctx: Context, token: string) => void; description: string; approvalUrl?: string; metadata?: Record<string, any> }): WorkflowInstance;
  waitForEvent(eventName: string, timeout?: string): WorkflowInstance;
  pause(pauseCallback: (ctx: Context) => void | Promise<void>): WorkflowInstance;
  log(message: string | ((ctx: Context) => string), level?: 'info' | 'warn' | 'error'): WorkflowInstance;
  
  // Testing
  test(): TestHarness;
  advancedTest(): AdvancedTestHarness;
  
  // Workflow info
  validate(): void;
  toJSON(): string;
  getDefinition(): WorkflowDefinition;
  getId(): string;
  getName(): string;
  getSteps(): StepDefinition[];
  getTriggers(): TriggerDefinition[];
  hasWebhookTriggers(): boolean;
  hasScheduleTriggers(): boolean;
  hasManualTriggers(): boolean;
  getStep(id: string): StepDefinition | undefined;
  register(): Promise<void>;
}

export declare class TestHarness {
  constructor();
  executeWorkflow(workflow: WorkflowDefinition, payload?: any): Promise<any>;
  mockStep(stepId: string, mockFn: (ctx: Context) => any): void;
  clearMocks(): void;
  getExecutionHistory(): any[];
}

export declare class AdvancedTestHarness extends TestHarness {
  timeTravel(timestamp: number): void;
  mockExternalService(serviceName: string, responses: any[]): void;
  simulateError(stepId: string, error: Error): void;
  captureEvents(): any[];
}

export declare class RetryExecutor {
  constructor(config: RetryConfig);
  execute<T>(operation: () => Promise<T>, operationName?: string): Promise<T>;
}

export declare class StepExecutor {
  constructor();
  execute(step: StepDefinition, context: Context): Promise<any>;
}

export declare class CircuitBreaker {
  constructor(options?: { failureThreshold?: number; recoveryTimeout?: number; expectedErrors?: string[] });
  execute<T>(operation: () => Promise<T>): Promise<T>;
  getState(): 'CLOSED' | 'OPEN' | 'HALF_OPEN';
  reset(): void;
}

export declare class CircuitBreakerManager {
  static getInstance(): CircuitBreakerManager;
  getBreaker(name: string, options?: any): CircuitBreaker;
  getAllBreakers(): Map<string, CircuitBreaker>;
  resetAll(): void;
}

export declare class PerformanceOptimizer {
  constructor();
  optimize(workflow: WorkflowDefinition): WorkflowDefinition;
  getOptimizationReport(): any;
}

export declare class PerformanceMonitor {
  constructor();
  startMonitoring(workflowId: string): void;
  stopMonitoring(workflowId: string): void;
  getMetrics(workflowId: string): any;
}

// ============================================================================
// Human Loop Types
// ============================================================================

export interface PausedWorkflow {
  id: string;
  workflowId: string;
  stepId: string;
  context: Context;
  token: string;
  description?: string;
  approvalUrl?: string;
  metadata?: Record<string, any>;
  createdAt: number;
  expiresAt?: number;
}

// ============================================================================
// Event System Types
// ============================================================================

export interface EventListener {
  workflowId: string;
  eventName: string;
  handler: (payload: any) => void | Promise<void>;
}

export interface EventHistoryItem {
  name: string;
  payload: any;
  timestamp: number;
}

// ============================================================================
// Hook System Types
// ============================================================================

export interface HookHandler {
  (ctx: Context, stepId?: string | string[]): void | Promise<void>;
}

// ============================================================================
// Benchmark Types
// ============================================================================

export interface BenchmarkOptions {
  iterations?: number;
  warmup?: number;
  concurrency?: number;
  payload?: any;
}

export interface BenchmarkResult {
  name: string;
  iterations: number;
  totalTime: number;
  averageTime: number;
  minTime: number;
  maxTime: number;
  opsPerSecond: number;
  concurrency: number;
}

// ============================================================================
// Core Functions
// ============================================================================

export interface StartOptions {
  webhookServer?: {
    host?: string;
    port?: number;
    maxConnections?: number;
  };
}

export declare function define(options: Omit<WorkflowDefinition, 'steps' | 'triggers' | 'created_at' | 'updated_at'>): WorkflowInstance;
export declare function start(options?: StartOptions): Promise<void>;
export declare function stop(): Promise<void>;
export declare function trigger(workflowId: string, payload?: any): Promise<string>;
export declare function inspect(runId: string): Promise<any>;
export declare function getWorkflows(): WorkflowDefinition[];
export declare function getWorkflow(workflowId: string): WorkflowDefinition | undefined;
export declare function getConcurrencyStats(workflowId?: string): any;

// State Management
export declare function getGlobalState(key: string, defaultValue?: any): Promise<any>;
export declare function setGlobalState(key: string, value: any, options?: { ttl?: string }): Promise<void>;
export declare function incrGlobalState(key: string, amount?: number): Promise<number>;
export declare function deleteGlobalState(key: string): Promise<boolean>;
export declare function getWorkflowState(workflowId: string, key: string, defaultValue?: any): Promise<any>;
export declare function setWorkflowState(workflowId: string, key: string, value: any, options?: { ttl?: string }): Promise<void>;
export declare function incrWorkflowState(workflowId: string, key: string, amount?: number): Promise<number>;
export declare function deleteWorkflowState(workflowId: string, key: string): Promise<boolean>;
export declare function getStateStats(): Promise<{
  global: {
    totalKeys: number;
    expiredKeys: number;
    namespace: string;
    dbPath: string;
  };
  workflows: Record<string, {
    totalKeys: number;
    expiredKeys: number;
    namespace: string;
    dbPath: string;
  }>;
}>;
export declare function cleanupExpiredState(): Promise<{
  global: number;
  workflows: Record<string, number>;
}>;

// Human Loop
export declare function resume(token: string, payload: any): Promise<void>;
export declare function storePausedWorkflow(token: string, pauseInfo: {
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
}): void;
export declare function getPausedWorkflow(token: string): any;
export declare function listPausedWorkflows(): any[];

// Events
export declare function publishEvent(eventName: string, payload: any): Promise<void>;
export declare function registerEventListener(eventName: string, workflowId: string, trigger: any): void;
export declare function unregisterEventListener(eventName: string, workflowId: string): void;
export declare function getEventHistory(eventName?: string): Array<{ name: string; payload: any; timestamp: number }>;
export declare function getEventListeners(eventName?: string): Map<string, Array<{ workflowId: string; trigger: any }>>;

// Hooks
export declare function executeWorkflowHook(hookType: string, contextJson: string, workflowId: string, stepId?: string): Promise<any>;

// Performance
export declare function benchmark(options?: BenchmarkOptions): Promise<BenchmarkResult>;

// Triggers
export declare function executeManualTrigger(workflowId: string, payload: any): Promise<any>;
export declare function executeWebhookTrigger(request: any): Promise<any>;
export declare function executeScheduleTrigger(triggerId: string): Promise<any>;
export declare function getTriggerStats(): Promise<any>;
export declare function getWorkflowTriggers(workflowId: string): Promise<any>;
export declare function unregisterWorkflowTriggers(workflowId: string): Promise<any>;
export declare function getScheduleTriggers(): Promise<any>;

// Testing
export declare function createTestHarness(): TestHarness;

// Rust Integration
export declare function isRustCoreAvailable(): boolean;
export declare function getCoreStatus(): any;

// Webhook Server
export declare function createWebhookServer(options?: { port?: number }): any;

// Execution
export declare function executeWorkflowSteps(workflow: WorkflowDefinition, context: Context): Promise<any>;
export declare function executeStep(step: StepDefinition, context: Context): Promise<any>;
export declare function executeStepFunction(stepFn: (ctx: Context) => any, context: Context): Promise<any>;
export declare function executeJobFunction(jobFn: (ctx: Context) => any, context: Context): Promise<any>;
export declare function createValidContext(payload?: any, workflowId?: string): Context;

// Scheduler
export declare const scheduler: {
  schedule(cronExpression: string, workflowId: string): void;
  unschedule(workflowId: string): void;
  getScheduledWorkflows(): any[];
  start(): void;
  stop(): void;
};

// Additional exported functions
export declare function cancelRun(runId: string): Promise<void>;
export declare function getEngineState(): 'STOPPED' | 'STARTING' | 'STARTED';
export declare function replay(runId: string, options?: { overridePayload?: any; mockStep?: (stepName: string, mockFn: (ctx: Context) => any) => void }): Promise<void>;
export declare function registerStepHandler(workflowId: string, stepId: string, handler: (ctx: Context) => any | Promise<any>, type: 'step' | 'action'): void;

// ============================================================================
// Main Cronflow Object
// ============================================================================

export declare const cronflow: {
  define: typeof define;
  start: typeof start;
  stop: typeof stop;
  trigger: typeof trigger;
  inspect: typeof inspect;
  cancelRun: typeof cancelRun;
  publishEvent: typeof publishEvent;
  executeStep: typeof executeStep;
  executeStepFunction: typeof executeStepFunction;
  executeJobFunction: typeof executeJobFunction;
  getWorkflows: typeof getWorkflows;
  getWorkflow: typeof getWorkflow;
  getEngineState: typeof getEngineState;
  replay: typeof replay;
  resume: typeof resume;
  isRustCoreAvailable: typeof isRustCoreAvailable;
  benchmark: typeof benchmark;
  executeManualTrigger: typeof executeManualTrigger;
  executeWebhookTrigger: typeof executeWebhookTrigger;
  executeScheduleTrigger: typeof executeScheduleTrigger;
  getTriggerStats: typeof getTriggerStats;
  getWorkflowTriggers: typeof getWorkflowTriggers;
  unregisterWorkflowTriggers: typeof unregisterWorkflowTriggers;
  getScheduleTriggers: typeof getScheduleTriggers;
  createValidContext: typeof createValidContext;
  executeWorkflowHook: typeof executeWorkflowHook;
  getGlobalState: typeof getGlobalState;
  setGlobalState: typeof setGlobalState;
  incrGlobalState: typeof incrGlobalState;
  deleteGlobalState: typeof deleteGlobalState;
  getWorkflowState: typeof getWorkflowState;
  setWorkflowState: typeof setWorkflowState;
  incrWorkflowState: typeof incrWorkflowState;
  deleteWorkflowState: typeof deleteWorkflowState;
  getStateStats: typeof getStateStats;
  cleanupExpiredState: typeof cleanupExpiredState;
  registerStepHandler: typeof registerStepHandler;
  VERSION: string;
  registerEventListener: typeof registerEventListener;
  unregisterEventListener: typeof unregisterEventListener;
  getEventHistory: typeof getEventHistory;
  getEventListeners: typeof getEventListeners;
  scheduler: typeof scheduler;
};
`,
};

// Write declaration files
for (const [filePath, content] of Object.entries(declarations)) {
  const fullPath = path.join(__dirname, '..', filePath);
  const dir = path.dirname(fullPath);

  // Ensure directory exists
  if (!fs.existsSync(dir)) {
    fs.mkdirSync(dir, { recursive: true });
  }

  fs.writeFileSync(fullPath, content);
  console.log(`✅ Generated: ${filePath}`);
}

console.log('🎉 All TypeScript declaration files generated!');
