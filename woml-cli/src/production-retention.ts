import { createHash, randomUUID } from 'node:crypto';
import { lstat } from 'node:fs/promises';
import { resolve } from 'node:path';

import {
  executeRetentionWithRust,
  executeRetentionWithRustAsync,
  planRetentionWithRust,
  type RetentionPolicyV1,
  type RustRetentionExecutionV1,
  type RustRetentionPlanV1,
} from './rust-executor';

export interface AutomaticRetentionConfiguration {
  readonly enabled: boolean;
  readonly succeededAfterDays?: number;
  readonly failedAfterDays?: number;
  readonly cancelledAfterDays?: number;
  readonly maintenanceHourUtc?: number;
}

export interface AutomaticRetentionHandle {
  readonly nextRunAt?: string;
  runNow(): Promise<RustRetentionExecutionV1 | undefined>;
  close(): void;
}

type RetentionTimer = number | ReturnType<typeof setTimeout>;

export const pruneUsage =
  'Usage: woml prune --before <duration> [--state <path>] [--dry-run] [--compact] [--json]';

const MAX_RETENTION_MILLISECONDS = 3_650 * 24 * 60 * 60 * 1_000;

export interface PruneArguments {
  readonly statePath: string;
  readonly before: string;
  readonly beforeMs: number;
  readonly dryRun: boolean;
  readonly compact: boolean;
  readonly json: boolean;
}

export class ProductionRetentionError extends Error {
  readonly code: string;

  constructor(code: string, message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = 'ProductionRetentionError';
    this.code = code;
  }
}

function duration(value: string): number {
  const match = /^(\d+)(h|d|w)$/.exec(value);
  if (match === null) {
    throw new ProductionRetentionError(
      'WOML_RETENTION_PLAN_INVALID',
      '--before must be a whole number followed by h, d, or w, such as 30d.'
    );
  }
  const amount = Number(match[1]);
  const unit = match[2] === 'h' ? 60 * 60 * 1_000 : match[2] === 'd'
    ? 24 * 60 * 60 * 1_000
    : 7 * 24 * 60 * 60 * 1_000;
  const milliseconds = amount * unit;
  if (
    !Number.isSafeInteger(milliseconds) ||
    milliseconds < 60 * 60 * 1_000 ||
    milliseconds > MAX_RETENTION_MILLISECONDS
  ) {
    throw new ProductionRetentionError(
      'WOML_RETENTION_PLAN_INVALID',
      '--before must be from 1h through 3650d.'
    );
  }
  return milliseconds;
}

export function parsePruneArguments(args: readonly string[]): PruneArguments {
  if (args[0] !== 'prune') {
    throw new ProductionRetentionError('WOML_CLI_ARGUMENTS_INVALID', pruneUsage);
  }
  let statePath = resolve('.woml/state.sqlite');
  let before: string | undefined;
  let dryRun = false;
  let compact = false;
  let json = false;
  const seen = new Set<string>();
  for (let index = 1; index < args.length; index += 1) {
    const option = args[index]!;
    if (seen.has(option)) {
      throw new ProductionRetentionError('WOML_CLI_ARGUMENTS_INVALID', pruneUsage);
    }
    seen.add(option);
    if (option === '--dry-run') {
      dryRun = true;
      continue;
    }
    if (option === '--compact') {
      compact = true;
      continue;
    }
    if (option === '--json') {
      json = true;
      continue;
    }
    const value = args[++index];
    if (
      value === undefined ||
      value.startsWith('--') ||
      !['--state', '--before'].includes(option)
    ) {
      throw new ProductionRetentionError('WOML_CLI_ARGUMENTS_INVALID', pruneUsage);
    }
    if (option === '--state') statePath = resolve(value);
    if (option === '--before') before = value;
  }
  if (before === undefined || (dryRun && compact)) {
    throw new ProductionRetentionError('WOML_CLI_ARGUMENTS_INVALID', pruneUsage);
  }
  return {
    statePath,
    before,
    beforeMs: duration(before),
    dryRun,
    compact,
    json,
  };
}

function policyId(subject: string): string {
  return `retention_${createHash('sha256').update(`woml.retention/v1\0${subject}`).digest('hex').slice(0, 24)}`;
}

export function retentionPolicy(
  now: Date,
  ages: {
    readonly succeededMs: number;
    readonly failedMs: number;
    readonly cancelledMs: number;
    readonly identity: string;
  }
): RetentionPolicyV1 {
  return {
    policyId: policyId(ages.identity),
    succeededBefore: new Date(now.getTime() - ages.succeededMs).toISOString(),
    failedBefore: new Date(now.getTime() - ages.failedMs).toISOString(),
    cancelledBefore: new Date(now.getTime() - ages.cancelledMs).toISOString(),
  };
}

async function assertStateDatabase(path: string): Promise<void> {
  const entry = await lstat(path).catch(() => undefined);
  if (entry === undefined || !entry.isFile() || entry.isSymbolicLink()) {
    throw new ProductionRetentionError(
      'WOML_RETENTION_STORE_INVALID',
      `No safe durable WOML state database exists at ${path}.`
    );
  }
}

export async function runProductionRetention(
  args: PruneArguments,
  options: { readonly nativeCorePath?: string; readonly now?: Date } = {}
): Promise<RustRetentionPlanV1 | RustRetentionExecutionV1> {
  await assertStateDatabase(args.statePath);
  const now = options.now ?? new Date();
  const policy = retentionPolicy(now, {
    succeededMs: args.beforeMs,
    failedMs: args.beforeMs,
    cancelledMs: args.beforeMs,
    identity: `cli:${args.before}`,
  });
  if (args.dryRun) {
    return planRetentionWithRust(args.statePath, policy, now.toISOString(), {
      nativeCorePath: options.nativeCorePath,
    });
  }
  return executeRetentionWithRust(
    args.statePath,
    policy,
    `lease_${randomUUID().replaceAll('-', '')}`,
    `prune_cli_${process.pid}_${randomUUID().replaceAll('-', '')}`,
    args.compact,
    now.toISOString(),
    { nativeCorePath: options.nativeCorePath }
  );
}

const DAY_MS = 24 * 60 * 60 * 1_000;

export function nextRetentionTime(now: Date, hourUtc: number): Date {
  const next = new Date(now);
  next.setUTCHours(hourUtc, 0, 0, 0);
  if (next.getTime() <= now.getTime()) next.setUTCDate(next.getUTCDate() + 1);
  return next;
}

export function startAutomaticRetention(options: {
  readonly statePath: string;
  readonly configuration?: AutomaticRetentionConfiguration;
  readonly nativeCorePath?: string;
  readonly ownerId: string;
  readonly now?: () => Date;
  readonly setTimer?: (callback: () => void, delay: number) => RetentionTimer;
  readonly clearTimer?: (timer: RetentionTimer) => void;
  readonly onResult?: (execution: RustRetentionExecutionV1) => void;
  readonly onError?: (error: unknown) => void;
}): AutomaticRetentionHandle {
  const configuration = options.configuration;
  if (configuration?.enabled !== true) {
    return { runNow: async () => undefined, close: () => {} };
  }
  const now = options.now ?? (() => new Date());
  const setTimer = options.setTimer ?? ((callback, delay) => setTimeout(callback, delay));
  const clearTimer = options.clearTimer ?? (timer => clearTimeout(timer));
  const hour = configuration.maintenanceHourUtc ?? 3;
  const next = nextRetentionTime(now(), hour);
  let timer: RetentionTimer | undefined;
  let closed = false;
  let active: Promise<RustRetentionExecutionV1 | undefined> | undefined;

  const runNow = (): Promise<RustRetentionExecutionV1 | undefined> => {
    if (closed) return Promise.resolve(undefined);
    if (active !== undefined) return active;
    const current = now();
    const policy = retentionPolicy(current, {
      succeededMs: (configuration.succeededAfterDays ?? 30) * DAY_MS,
      failedMs: (configuration.failedAfterDays ?? 90) * DAY_MS,
      cancelledMs: (configuration.cancelledAfterDays ?? 30) * DAY_MS,
      identity: `automatic:${configuration.succeededAfterDays ?? 30}:${configuration.failedAfterDays ?? 90}:${configuration.cancelledAfterDays ?? 30}`,
    });
    active = executeRetentionWithRustAsync(
      options.statePath,
      policy,
      `lease_${randomUUID().replaceAll('-', '')}`,
      options.ownerId,
      false,
      current.toISOString(),
      { nativeCorePath: options.nativeCorePath }
    )
      .then(execution => {
        options.onResult?.(execution);
        return execution;
      })
      .catch(error => {
        options.onError?.(error);
        return undefined;
      })
      .finally(() => {
        active = undefined;
      });
    return active;
  };

  const schedule = (): void => {
    if (closed) return;
    const scheduled = nextRetentionTime(now(), hour);
    timer = setTimer(() => {
      void runNow().finally(schedule);
    }, Math.max(1, scheduled.getTime() - now().getTime()));
  };
  schedule();
  return {
    nextRunAt: next.toISOString(),
    runNow,
    close: () => {
      closed = true;
      if (timer !== undefined) clearTimer(timer);
    },
  };
}
