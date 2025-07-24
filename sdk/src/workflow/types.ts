import { z } from 'zod';

export interface ConfiguredService {
  id: string;
  name: string;
  version: string;
  config: any;
  auth: any;
  actions: Record<string, (...args: any[]) => any>;
  triggers?: Record<string, (...args: any[]) => any>;
}

export interface WorkflowDefinition {
  id: string;
  name?: string;
  description?: string;
  tags?: string[];
  services?: ConfiguredService[];
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

export interface StepDefinition {
  id: string;
  name: string;
  handler: (ctx: Context) => any | Promise<any>;
  type: 'step' | 'action';
  options?: StepOptions;
  parallel?: boolean;
  parallel_group_id?: string;
  parallel_step_count?: number;
  race?: boolean;
  for_each?: boolean;
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
  background?: boolean; // Whether this step should run as a background side effect
}

export interface RetryConfig {
  attempts: number;
  backoff: {
    strategy: 'exponential' | 'fixed';
    delay: string | number;
  };
  jitter?: boolean;
  maxBackoff?: string | number;
  onRetry?: (attempt: number, error: Error, delay: number) => void;
  shouldRetry?: (error: Error) => boolean;
}

export interface CacheConfig {
  key: (ctx: Context) => string;
  ttl: string;
}

export type TriggerDefinition =
  | { type: 'webhook'; path: string; options?: WebhookOptions }
  | { type: 'schedule'; cron_expression: string }
  | { type: 'manual' };

export interface WebhookOptions {
  method?: 'POST' | 'GET' | 'PUT' | 'DELETE';
  schema?: z.ZodObject<any>;
  idempotencyKey?: (ctx: Context) => string;
  parseRawBody?: boolean;
  headers?: {
    required?: Record<string, string>; // e.g., { 'content-type': 'application/json' }
    validate?: (headers: Record<string, string>) => boolean | string;
  };
}

// Context object that gets passed to step handlers
export interface Context {
  payload: any;
  steps: Record<string, { output: any }>;
  services: Record<string, ConfiguredService>;
  run: {
    id: string;
    workflowId: string;
  };
  state: {
    get: (key: string, defaultValue?: any) => any;
    set: (key: string, value: any, options?: { ttl?: string }) => Promise<void>;
    incr: (key: string, amount?: number) => Promise<number>;
  };
  last: any; // Output from the previous step
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
}
