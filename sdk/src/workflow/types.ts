import { z } from 'zod';
import { SupportedFramework } from './framework-registry';

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

export interface StepConfig {
  id: string;
  title?: string;
  description?: string;
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
  | { type: 'event'; eventName: string }
  | { type: 'manual' };

export interface WebhookOptions {
  method?: 'POST' | 'GET' | 'PUT' | 'DELETE';
  schema?: z.ZodObject<any>;
  idempotencyKey?: (ctx: Context) => string;
  parseRawBody?: boolean;
  app?: SupportedFramework;
  appInstance?: any;
  registerRoute?: (method: string, path: string, handler: Function) => void;
  headers?: {
    required?: Record<string, string>;
    validate?: (headers: Record<string, string>) => boolean | string;
  };
  trigger?: string;
}

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
