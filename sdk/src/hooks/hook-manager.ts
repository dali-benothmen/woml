export type HookHandler = (
  hookType: string,
  contextJson: string,
  workflowId: string,
  stepId?: string
) => Promise<any>;

let hookHandlers: Map<string, HookHandler> = new Map();
let getWorkflowFunction: (id: string) => any = () => undefined;

export function setHookHandlers(handlers: Map<string, HookHandler>): void {
  hookHandlers = handlers;
}

export function getHookHandlers(): Map<string, HookHandler> {
  return hookHandlers;
}

export function setGetWorkflowFunction(fn: (id: string) => any): void {
  getWorkflowFunction = fn;
}

export function registerHookHandler(
  hookType: 'onSuccess' | 'onFailure',
  handler: HookHandler
): void {
  hookHandlers.set(hookType, handler);
}

export function getHookHandler(hookType: string): HookHandler | undefined {
  return hookHandlers.get(hookType);
}

export async function executeWorkflowHook(
  hookType: string,
  contextJson: string,
  workflowId: string,
  stepId?: string
): Promise<any> {
  try {
    const context = JSON.parse(contextJson);

    const workflow = getWorkflowFunction(workflowId);
    if (!workflow) {
      return {
        success: true,
        message: `No ${hookType} hook defined (workflow not found)`,
      };
    }

    const hook =
      hookType === 'onSuccess'
        ? workflow.hooks?.onSuccess
        : workflow.hooks?.onFailure;

    if (hook) {
      try {
        await hook(context, stepId);
        return {
          success: true,
          message: `${hookType} hook executed successfully`,
          hookType,
          workflowId,
          stepId,
        };
      } catch (error: any) {
        return {
          success: false,
          error: error.message,
          hookType,
          workflowId,
          stepId,
        };
      }
    } else {
      return {
        success: true,
        message: `No ${hookType} hook defined`,
        hookType,
        workflowId,
        stepId,
      };
    }
  } catch (error: any) {
    return {
      success: false,
      error: error.message,
      hookType,
      workflowId,
      stepId,
    };
  }
}
