interface WorkflowExecution {
  workflowId: string;
  runId: string;
  startTime: number;
  payload: any;
}

interface ConcurrencyConfig {
  maxConcurrent: number;
  queue: WorkflowExecution[];
  running: Map<string, WorkflowExecution>;
}

class ConcurrencyManager {
  private workflowConcurrency: Map<string, ConcurrencyConfig> = new Map();
  private executionQueue: Map<string, Promise<any>> = new Map();

  registerWorkflow(workflowId: string, maxConcurrent: number): void {
    if (maxConcurrent <= 0) {
      console.warn(
        `Invalid concurrency limit ${maxConcurrent} for workflow ${workflowId}, using 1`
      );
      maxConcurrent = 1;
    }

    this.workflowConcurrency.set(workflowId, {
      maxConcurrent,
      queue: [],
      running: new Map(),
    });
  }

  canExecute(workflowId: string): boolean {
    const config = this.workflowConcurrency.get(workflowId);
    if (!config) {
      return true;
    }

    return config.running.size < config.maxConcurrent;
  }

  getCurrentExecutions(workflowId: string): number {
    const config = this.workflowConcurrency.get(workflowId);
    return config ? config.running.size : 0;
  }

  getQueueLength(workflowId: string): number {
    const config = this.workflowConcurrency.get(workflowId);
    return config ? config.queue.length : 0;
  }

  async executeWithConcurrency<T>(
    workflowId: string,
    runId: string,
    payload: any,
    executeFunction: () => Promise<T>
  ): Promise<T> {
    const config = this.workflowConcurrency.get(workflowId);

    if (!config) {
      return await executeFunction();
    }

    const execution: WorkflowExecution = {
      workflowId,
      runId,
      startTime: Date.now(),
      payload,
    };

    if (this.canExecute(workflowId)) {
      return await this._executeImmediately(config, execution, executeFunction);
    } else {
      return await this._queueExecution(config, execution, executeFunction);
    }
  }

  private async _executeImmediately<T>(
    config: ConcurrencyConfig,
    execution: WorkflowExecution,
    executeFunction: () => Promise<T>
  ): Promise<T> {
    config.running.set(execution.runId, execution);

    try {
      const result = await executeFunction();
      return result;
    } finally {
      config.running.delete(execution.runId);
      this._processQueue(config);
    }
  }

  private async _queueExecution<T>(
    config: ConcurrencyConfig,
    execution: WorkflowExecution,
    executeFunction: () => Promise<T>
  ): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      const queuedExecution = {
        ...execution,
        execute: async () => {
          try {
            config.running.set(execution.runId, execution);
            const result = await executeFunction();
            resolve(result);
          } catch (error) {
            reject(error);
          } finally {
            config.running.delete(execution.runId);
            this._processQueue(config);
          }
        },
      };

      config.queue.push(queuedExecution as any);
    });
  }

  private _processQueue(config: ConcurrencyConfig): void {
    if (
      config.queue.length === 0 ||
      config.running.size >= config.maxConcurrent
    ) {
      return;
    }

    const next = config.queue.shift();
    if (next && (next as any).execute) {
      setTimeout(() => (next as any).execute(), 0);
    }
  }

  stopExecution(workflowId: string, runId: string): boolean {
    const config = this.workflowConcurrency.get(workflowId);
    if (!config) {
      return false;
    }

    const wasRunning = config.running.delete(runId);

    const queueIndex = config.queue.findIndex(exec => exec.runId === runId);
    if (queueIndex >= 0) {
      config.queue.splice(queueIndex, 1);
    }

    if (wasRunning) {
      this._processQueue(config);
    }

    return wasRunning || queueIndex >= 0;
  }

  getStats(workflowId: string): {
    maxConcurrent: number;
    running: number;
    queued: number;
    runningExecutions: WorkflowExecution[];
    queuedExecutions: WorkflowExecution[];
  } | null {
    const config = this.workflowConcurrency.get(workflowId);
    if (!config) {
      return null;
    }

    return {
      maxConcurrent: config.maxConcurrent,
      running: config.running.size,
      queued: config.queue.length,
      runningExecutions: Array.from(config.running.values()),
      queuedExecutions: config.queue,
    };
  }

  getAllStats(): Record<string, ReturnType<typeof this.getStats>> {
    const stats: Record<string, ReturnType<typeof this.getStats>> = {};

    for (const [workflowId] of this.workflowConcurrency) {
      stats[workflowId] = this.getStats(workflowId);
    }

    return stats;
  }

  clear(): void {
    this.workflowConcurrency.clear();
    this.executionQueue.clear();
  }

  updateConcurrency(workflowId: string, maxConcurrent: number): void {
    const config = this.workflowConcurrency.get(workflowId);
    if (!config) {
      this.registerWorkflow(workflowId, maxConcurrent);
      return;
    }

    const oldLimit = config.maxConcurrent;
    config.maxConcurrent = Math.max(1, maxConcurrent);

    if (config.maxConcurrent > oldLimit) {
      this._processQueue(config);
    }
  }
}

export const concurrencyManager = new ConcurrencyManager();

export default concurrencyManager;
