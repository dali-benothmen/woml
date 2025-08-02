import { createStateManager } from './manager';

export async function getWorkflowState(
  workflowId: string,
  key: string,
  getCurrentState: () => any,
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
  getCurrentState: () => any,
  options?: { ttl?: string }
): Promise<void> {
  const currentState = getCurrentState();
  const stateManager = createStateManager(workflowId, currentState.dbPath);
  await stateManager.set(key, value, options);
}

export async function incrWorkflowState(
  workflowId: string,
  key: string,
  getCurrentState: () => any,
  amount: number = 1
): Promise<number> {
  const currentState = getCurrentState();
  const stateManager = createStateManager(workflowId, currentState.dbPath);
  return await stateManager.incr(key, amount);
}

export async function deleteWorkflowState(
  workflowId: string,
  key: string,
  getCurrentState: () => any
): Promise<boolean> {
  const currentState = getCurrentState();
  const stateManager = createStateManager(workflowId, currentState.dbPath);
  return await stateManager.delete(key);
}
