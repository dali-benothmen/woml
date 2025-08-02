import { loadCoreModule } from '../utils/core-resolver';

const { core } = loadCoreModule();

let getCurrentStateFunction: () => any = () => ({});

export function setTriggerManagerState(getCurrentState: () => any): void {
  getCurrentStateFunction = getCurrentState;
}

export async function executeManualTrigger(
  workflowId: string,
  payload: any
): Promise<any> {
  if (!core) {
    return {
      success: true,
      run_id: 'simulation-run-id',
      workflow_id: workflowId,
      message: 'Manual trigger executed in simulation mode',
    };
  }

  try {
    const payloadJson = JSON.stringify(payload);
    const result = core.executeManualTrigger(
      workflowId,
      payloadJson,
      getCurrentStateFunction().dbPath
    );

    if (result.success) {
      return result;
    } else {
      throw new Error(`Failed to execute manual trigger: ${result.message}`);
    }
  } catch (error) {
    throw error;
  }
}

export async function executeWebhookTrigger(request: any): Promise<any> {
  if (!core) {
    return {
      success: true,
      run_id: 'simulation-webhook-run-id',
      workflow_id: 'simulation-workflow',
      message: 'Webhook trigger executed in simulation mode',
    };
  }

  try {
    const requestJson = JSON.stringify(request);
    const result = core.executeWebhookTrigger(
      requestJson,
      getCurrentStateFunction().dbPath
    );

    if (result.success) {
      return result;
    } else {
      throw new Error(`Failed to execute webhook trigger: ${result.message}`);
    }
  } catch (error) {
    throw error;
  }
}

export async function executeScheduleTrigger(triggerId: string): Promise<any> {
  if (!core) {
    return {
      success: true,
      run_id: 'simulation-schedule-run-id',
      workflow_id: 'simulation-workflow',
      message: 'Schedule trigger executed in simulation mode',
    };
  }

  try {
    const result = core.executeScheduleTrigger(
      triggerId,
      getCurrentStateFunction().dbPath
    );

    if (result.success) {
      return result;
    } else {
      throw new Error(`Failed to execute schedule trigger: ${result.message}`);
    }
  } catch (error) {
    throw error;
  }
}

export async function getTriggerStats(): Promise<any> {
  if (!core) {
    return {
      success: true,
      stats: JSON.stringify({
        total_triggers: 0,
        webhook_triggers: 0,
        schedule_triggers: 0,
      }),
      message: 'Trigger statistics retrieved in simulation mode',
    };
  }

  try {
    const result = core.getTriggerStats(getCurrentStateFunction().dbPath);

    if (result.success) {
      return result;
    } else {
      throw new Error(`Failed to get trigger stats: ${result.message}`);
    }
  } catch (error) {
    throw error;
  }
}

export async function getWorkflowTriggers(workflowId: string): Promise<any> {
  if (!core) {
    return {
      success: true,
      triggers: JSON.stringify(['manual']),
      message: 'Workflow triggers retrieved in simulation mode',
    };
  }

  try {
    const result = core.getWorkflowTriggers(
      workflowId,
      getCurrentStateFunction().dbPath
    );

    if (result.success) {
      return result;
    } else {
      throw new Error(`Failed to get workflow triggers: ${result.message}`);
    }
  } catch (error) {
    throw error;
  }
}

export async function unregisterWorkflowTriggers(
  workflowId: string
): Promise<any> {
  if (!core) {
    return {
      success: true,
      message: 'Workflow triggers unregistered in simulation mode',
    };
  }

  try {
    const result = core.unregisterWorkflowTriggers(
      workflowId,
      getCurrentStateFunction().dbPath
    );

    if (result.success) {
      return result;
    } else {
      throw new Error(
        `Failed to unregister workflow triggers: ${result.message}`
      );
    }
  } catch (error) {
    throw error;
  }
}

export async function getScheduleTriggers(): Promise<any> {
  if (!core) {
    return {
      success: true,
      triggers: JSON.stringify([]),
      message: 'Schedule triggers retrieved in simulation mode',
    };
  }

  try {
    const result = core.getScheduleTriggers(getCurrentStateFunction().dbPath);

    if (result.success) {
      return result;
    } else {
      throw new Error(`Failed to get schedule triggers: ${result.message}`);
    }
  } catch (error) {
    throw error;
  }
}
