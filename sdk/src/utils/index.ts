import type { Context } from '../workflow/types';
import { createStateWrapper } from '../state';
export * from './core-resolver';

/**
 * Convert interval string to cron expression
 * @param interval - The interval string (e.g., "5m", "1h", "1d")
 * @returns The corresponding cron expression
 * @throws {Error} When interval format is invalid
 */
export function intervalToCron(interval: string): string {
  const match = interval.match(/^(\d+)([mhd])$/);
  if (!match) {
    throw new Error(
      `Invalid interval format: ${interval}. Expected format like "5m", "1h", "1d"`
    );
  }

  const [, amount, unit] = match;
  const num = parseInt(amount);

  switch (unit) {
    case 'm': // minutes
      return `*/${num} * * * *`;
    case 'h': // hours
      return `0 */${num} * * *`;
    case 'd': // days
      return `0 0 */${num} * *`;
    default:
      throw new Error(`Unsupported interval unit: ${unit}`);
  }
}

/**
 * Parse duration string or number to milliseconds
 * @param duration - The duration string (e.g., "5s", "1m", "1h", "1d") or number
 * @returns The duration in milliseconds
 * @throws {Error} When duration format is invalid
 */
export function parseDuration(duration: string | number): number {
  if (typeof duration === 'number') {
    return duration;
  }

  const units: Record<string, number> = {
    ms: 1,
    s: 1000,
    m: 60 * 1000,
    h: 60 * 60 * 1000,
    d: 24 * 60 * 60 * 1000,
  };

  const match = duration.match(/^(\d+)(ms|s|m|h|d)$/);
  if (!match) {
    throw new Error(
      `Invalid duration format: ${duration}. Use format like "5s", "10m", "2h", "1d"`
    );
  }

  const [, value, unit] = match;
  return parseInt(value) * units[unit];
}

/**
 * Generate a unique ID for workflows, steps, or runs
 * @param prefix - Optional prefix for the ID (default: 'run')
 * @returns A unique string ID
 */
export function generateId(prefix: string = 'run'): string {
  return `${prefix}_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;
}

/**
 * Create a context object for workflow execution
 * @param payload - The workflow payload
 * @param workflowId - The workflow ID
 * @param runId - The run ID
 * @param steps - Record of step outputs
 * @param lastOutput - Output from the previous step
 * @param trigger - Trigger information
 * @param dbPath - Database path for state persistence
 * @returns A complete context object
 */
export function createContext(
  payload: any,
  workflowId: string,
  runId: string,
  steps: Record<string, { output: any }> = {},
  lastOutput: any = null,
  trigger: { headers: Record<string, string>; rawBody?: Buffer } = {
    headers: {},
  },
  dbPath: string = './cronflow.db'
): Context {
  // Create state wrapper for persistent state management
  const stateWrapper = createStateWrapper(workflowId, runId, dbPath);

  return {
    payload,
    steps,
    run: {
      id: runId,
      workflowId,
    },
    state: {
      get: (key: string, defaultValue?: any) => {
        return stateWrapper.get(key, defaultValue);
      },
      set: async (key: string, value: any, options?: { ttl?: string }) => {
        await stateWrapper.set(key, value, options);
      },
      incr: async (key: string, amount: number = 1) => {
        return await stateWrapper.incr(key, amount);
      },
    },
    last: lastOutput,
    trigger,
    cancel: (reason?: string) => {
      throw new Error(`Workflow cancelled: ${reason || 'No reason provided'}`);
    },
  };
}

/**
 * Deep clone an object (for workflow definitions)
 * @param obj - The object to clone
 * @returns A deep copy of the object
 */
export function deepClone<T>(obj: T): T {
  return JSON.parse(JSON.stringify(obj));
}

/**
 * Check if a value is a valid JSON
 * @param value - The value to check
 * @returns True if the value can be serialized to JSON
 */
export function isValidJSON(value: any): boolean {
  try {
    JSON.stringify(value);
    return true;
  } catch {
    return false;
  }
}

/**
 * Format a date for display
 * @param date - The date to format
 * @returns ISO string representation of the date
 */
export function formatDate(date: Date): string {
  return date.toISOString();
}

/**
 * Get the current timestamp
 * @returns Current timestamp in milliseconds
 */
export function getCurrentTimestamp(): number {
  return Date.now();
}
