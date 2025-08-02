import { createStateManager } from './manager';

export async function getGlobalState(
  key: string,
  getCurrentState: () => any,
  defaultValue?: any
): Promise<any> {
  const currentState = getCurrentState();
  const stateManager = createStateManager('global', currentState.dbPath);
  return await stateManager.get(key, defaultValue);
}

export async function setGlobalState(
  key: string,
  value: any,
  getCurrentState: () => any,
  options?: { ttl?: string }
): Promise<void> {
  const currentState = getCurrentState();
  const stateManager = createStateManager('global', currentState.dbPath);
  await stateManager.set(key, value, options);
}

export async function incrGlobalState(
  key: string,
  getCurrentState: () => any,
  amount: number = 1
): Promise<number> {
  const currentState = getCurrentState();
  const stateManager = createStateManager('global', currentState.dbPath);
  return await stateManager.incr(key, amount);
}

export async function deleteGlobalState(
  key: string,
  getCurrentState: () => any
): Promise<boolean> {
  const currentState = getCurrentState();
  const stateManager = createStateManager('global', currentState.dbPath);
  return await stateManager.delete(key);
}

export async function getStateStats(getCurrentState: () => any): Promise<{
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
  const currentState = getCurrentState();
  const globalStateManager = createStateManager('global', currentState.dbPath);
  const globalStats = await globalStateManager.stats();

  const workflowStats: Record<string, any> = {};
  for (const workflow of currentState.workflows.values()) {
    const workflowStateManager = createStateManager(
      workflow.id,
      currentState.dbPath
    );
    workflowStats[workflow.id] = await workflowStateManager.stats();
  }

  return {
    global: globalStats,
    workflows: workflowStats,
  };
}

export async function cleanupExpiredState(getCurrentState: () => any): Promise<{
  global: number;
  workflows: Record<string, number>;
}> {
  const currentState = getCurrentState();
  const globalStateManager = createStateManager('global', currentState.dbPath);
  const globalCleaned = await globalStateManager.cleanup();

  const workflowCleaned: Record<string, number> = {};
  for (const workflow of currentState.workflows.values()) {
    const workflowStateManager = createStateManager(
      workflow.id,
      currentState.dbPath
    );
    workflowCleaned[workflow.id] = await workflowStateManager.cleanup();
  }

  return {
    global: globalCleaned,
    workflows: workflowCleaned,
  };
}
