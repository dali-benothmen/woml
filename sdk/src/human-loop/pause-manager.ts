import { executeWorkflowSteps } from '../execution/workflow-engine';

export interface PausedWorkflow {
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
  resumedAt?: number;
}

const pausedWorkflows = new Map<string, PausedWorkflow>();

export async function resume(token: string, payload: any): Promise<void> {
  const pausedWorkflow = pausedWorkflows.get(token);

  if (!pausedWorkflow) {
    throw new Error(`No paused workflow found with token: ${token}`);
  }

  if (pausedWorkflow.status !== 'waiting') {
    throw new Error(
      `Workflow with token ${token} has already been ${pausedWorkflow.status}`
    );
  }

  if (pausedWorkflow.expiresAt && Date.now() > pausedWorkflow.expiresAt) {
    pausedWorkflow.status = 'timeout';
    throw new Error(`Workflow with token ${token} has expired`);
  }

  pausedWorkflow.status = 'resumed';
  pausedWorkflow.resumedAt = Date.now();

  if (pausedWorkflow.resumeCallback) {
    pausedWorkflow.resumeCallback(payload);
  } else {
    try {
      await executeWorkflowSteps(
        pausedWorkflow.workflowId,
        pausedWorkflow.runId,
        pausedWorkflow.payload
      );
    } catch (error) {
      throw error;
    }
  }

  pausedWorkflows.delete(token);

  return Promise.resolve();
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
  pausedWorkflows.set(token, pauseInfo);
}

export function getPausedWorkflow(token: string): PausedWorkflow | undefined {
  return pausedWorkflows.get(token);
}

export function listPausedWorkflows(): PausedWorkflow[] {
  return Array.from(pausedWorkflows.values());
}
