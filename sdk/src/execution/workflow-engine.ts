import { Context } from '../workflow/types';
import { loadCoreModule } from '../utils/core-resolver';

const { core } = loadCoreModule();

let stepRegistry: Map<string, any> = new Map();
let stateManager: any = null;

export function setWorkflowEngineState(newState: any): void {
  stateManager = newState;
}

export function setStepRegistry(registry: Map<string, any>): void {
  stepRegistry = registry;
}

export function setHookHandler(handler: any): void {
  // This will be set by cronflow.ts
}

function getCurrentState(): any {
  return stateManager;
}

function getStepHandler(workflowId: string, stepId: string): any {
  const key = `${workflowId}:${stepId}`;
  return stepRegistry.get(key);
}

function getHookHandler(hookType: string): any {
  return undefined;
}

function storePausedWorkflow(token: string, pauseInfo: any): void {
  // Implementation will be moved from cronflow.ts
}

export async function executeWorkflowSteps(
  workflowId: string,
  runId: string,
  payload: any
): Promise<void> {
  const currentState = getCurrentState();

  if (!core) {
    return;
  }

  try {
    const workflow = currentState.workflows.get(workflowId);
    if (!workflow) {
      throw new Error(`Workflow ${workflowId} not found`);
    }

    const completedSteps: Record<string, any> = {};
    let currentConditionMet = false;
    let inControlFlowBlock = false;
    let skipUntilEndIf = false;
    let lastExecutedStepIndex = -1;

    for (let i = 0; i < workflow.steps.length; i++) {
      const step = workflow.steps[i];

      const isControlFlowStep =
        step.options?.conditionType ||
        step.name.startsWith('conditional_') ||
        step.name.startsWith('else_') ||
        step.name.startsWith('endif_');

      if (isControlFlowStep) {
        if (
          step.name.startsWith('conditional_') ||
          step.options?.conditionType === 'if'
        ) {
          const context = {
            run_id: runId,
            workflow_id: workflowId,
            step_name: step.name,
            payload: payload,
            steps: completedSteps,
            services: {},
            run: {
              id: runId,
              workflowId: workflowId,
            },
            last:
              lastExecutedStepIndex >= 0
                ? completedSteps[workflow.steps[lastExecutedStepIndex].name]
                : undefined,
            state: {
              get: () => null,
              set: async () => {},
              incr: async () => 0,
            },
            trigger: { headers: {} },
            cancel: (reason?: string) => {
              throw new Error(
                `Workflow cancelled: ${reason || 'No reason provided'}`
              );
            },
          };

          const stepHandler = getStepHandler(workflowId, step.name);
          if (stepHandler) {
            const conditionResult = await stepHandler.handler(context);
            currentConditionMet = Boolean(conditionResult);

            if (currentConditionMet) {
              inControlFlowBlock = true;
              skipUntilEndIf = false;
            } else {
              skipUntilEndIf = true;
            }
          } else {
            skipUntilEndIf = true;
          }

          continue;
        } else if (
          step.name.startsWith('else_') ||
          step.options?.conditionType === 'else'
        ) {
          if (!currentConditionMet && !skipUntilEndIf) {
            inControlFlowBlock = true;
            skipUntilEndIf = false;
          } else {
            skipUntilEndIf = true;
          }

          continue;
        } else if (step.name.startsWith('endif_')) {
          inControlFlowBlock = false;
          skipUntilEndIf = false;
          currentConditionMet = false;

          continue;
        }
      }

      if (skipUntilEndIf) {
        continue;
      }

      if (step.options?.pause || step.name === 'pause') {
        const context = {
          run_id: runId,
          workflow_id: workflowId,
          step_name: step.name,
          payload: payload,
          steps: completedSteps,
          services: {},
          run: {
            id: runId,
            workflowId: workflowId,
          },
          last:
            lastExecutedStepIndex >= 0
              ? completedSteps[workflow.steps[lastExecutedStepIndex].name]
              : undefined,
          state: {
            get: () => null,
            set: async () => {},
            incr: async () => 0,
          },
          trigger: { headers: {} },
          cancel: (reason?: string) => {
            throw new Error(
              `Workflow cancelled: ${reason || 'No reason provided'}`
            );
          },
        };

        const contextJson = JSON.stringify(context);

        const stepResult = await executeStepFunction(
          step.name,
          contextJson,
          workflowId,
          runId
        );

        if (stepResult.success && stepResult.result) {
          completedSteps[step.name] = stepResult.result.output;
          lastExecutedStepIndex = i;

          const pauseInfo = {
            token: `pause_${runId}_${Date.now()}`,
            workflowId: workflowId,
            runId: runId,
            stepId: step.name,
            description: 'Workflow paused for manual intervention',
            metadata: {},
            createdAt: Date.now(),
            status: 'waiting' as const,
            payload: payload,
            lastStepOutput: stepResult.result.output,
          };

          storePausedWorkflow(pauseInfo.token, pauseInfo);

          return;
        } else {
          throw new Error(
            `Pause step ${step.name} failed: ${stepResult.message}`
          );
        }
      }

      if (step.options?.parallel || step.parallel) {
        const context = {
          run_id: runId,
          workflow_id: workflowId,
          step_name: step.name,
          payload: payload,
          steps: completedSteps,
          services: {},
          run: {
            id: runId,
            workflowId: workflowId,
          },
          last:
            lastExecutedStepIndex >= 0
              ? completedSteps[workflow.steps[lastExecutedStepIndex].name]
              : undefined,
          state: {
            get: () => null,
            set: async () => {},
            incr: async () => 0,
          },
          trigger: { headers: {} },
          cancel: (reason?: string) => {
            throw new Error(
              `Workflow cancelled: ${reason || 'No reason provided'}`
            );
          },
        };

        const contextJson = JSON.stringify(context);

        const stepResult = await executeStepFunction(
          step.name,
          contextJson,
          workflowId,
          runId
        );

        if (stepResult.success && stepResult.result) {
          completedSteps[step.name] = stepResult.result.output;
          lastExecutedStepIndex = i;
        } else {
          throw new Error(
            `Parallel step ${step.name} failed: ${stepResult.message}`
          );
        }
        continue;
      }

      const context = {
        run_id: runId,
        workflow_id: workflowId,
        step_name: step.name,
        payload: payload,
        steps: completedSteps,
        services: {},
        run: {
          id: runId,
          workflowId: workflowId,
        },
        last:
          lastExecutedStepIndex >= 0
            ? completedSteps[workflow.steps[lastExecutedStepIndex].name]
            : undefined,
        state: {
          get: () => null,
          set: async () => {},
          incr: async () => 0,
        },
        trigger: { headers: {} },
        cancel: (reason?: string) => {
          throw new Error(
            `Workflow cancelled: ${reason || 'No reason provided'}`
          );
        },
      };

      const contextJson = JSON.stringify(context);

      if (step.type === 'action' || step.options?.background) {
        executeStepFunction(step.name, contextJson, workflowId, runId)
          .then(stepResult => {
            if (stepResult.success && stepResult.result) {
              const onSuccessHandler = getHookHandler('onSuccess');
              if (onSuccessHandler) {
                try {
                  const stepContext = {
                    ...context,
                    step_name: step.name,
                    step_result: stepResult.result.output,
                    step_status: 'completed',
                    background: true,
                  };
                  onSuccessHandler(
                    'onSuccess',
                    JSON.stringify(stepContext),
                    workflowId,
                    step.name
                  ).catch((_hookError: any) => {});
                } catch (_hookError: any) {}
              }
            } else {
              const onFailureHandler = getHookHandler('onFailure');
              if (onFailureHandler) {
                try {
                  const stepContext = {
                    ...context,
                    step_name: step.name,
                    step_error: stepResult.message,
                    step_status: 'failed',
                    background: true,
                  };
                  onFailureHandler(
                    'onFailure',
                    JSON.stringify(stepContext),
                    workflowId,
                    step.name
                  ).catch((_hookError: any) => {});
                } catch (_hookError: any) {}
              }
            }
          })
          .catch((error: any) => {
            const onFailureHandler = getHookHandler('onFailure');
            if (onFailureHandler) {
              try {
                const stepContext = {
                  ...context,
                  step_name: step.name,
                  step_error: error.message || error.toString(),
                  step_status: 'failed',
                  background: true,
                };
                onFailureHandler(
                  'onFailure',
                  JSON.stringify(stepContext),
                  workflowId,
                  step.name
                ).catch((_hookError: any) => {});
              } catch (_hookError: any) {}
            }
          });

        continue;
      }

      const stepResult = await executeStepFunction(
        step.name,
        contextJson,
        workflowId,
        runId
      );

      if (stepResult.success && stepResult.result) {
        completedSteps[step.name] = stepResult.result.output;
        lastExecutedStepIndex = i;

        const onSuccessHandler = getHookHandler('onSuccess');
        if (onSuccessHandler) {
          try {
            const stepContext = {
              ...context,
              step_name: step.name,
              step_result: stepResult.result.output,
              step_status: 'completed',
            };
            await onSuccessHandler(
              'onSuccess',
              JSON.stringify(stepContext),
              workflowId,
              step.name
            );
          } catch (hookError) {}
        }
      } else {
        const onFailureHandler = getHookHandler('onFailure');
        if (onFailureHandler) {
          try {
            const stepContext = {
              ...context,
              step_name: step.name,
              step_error: stepResult.message,
              step_status: 'failed',
            };
            await onFailureHandler(
              'onFailure',
              JSON.stringify(stepContext),
              workflowId,
              step.name
            );
          } catch (hookError) {}
        }

        throw new Error(`Step ${step.name} failed: ${stepResult.message}`);
      }
    }
  } catch (error) {
    throw error;
  }
}

export async function executeStep(
  runId: string,
  stepId: string,
  contextJson?: string
): Promise<any> {
  if (!core) {
    return {
      success: true,
      result: {
        run_id: runId,
        step_id: stepId,
        status: 'simulation',
        message: 'Step executed in simulation mode',
      },
      message: 'Step executed successfully in simulation mode',
    };
  }

  try {
    const result = core.executeStep(
      runId,
      stepId,
      getCurrentState().dbPath,
      '{}'
    );

    if (result.success && result.result) {
      return JSON.parse(result.result);
    } else {
      throw new Error(`Failed to execute step: ${result.message}`);
    }
  } catch (error) {
    throw error;
  }
}

export async function executeStepFunction(
  stepName: string,
  contextJson: string,
  workflowId: string,
  runId: string
): Promise<any> {
  if (!core) {
    return {
      success: true,
      result: {
        step_name: stepName,
        workflow_id: workflowId,
        run_id: runId,
        status: 'simulation',
        message: 'Step function executed in simulation mode',
      },
      message: 'Step function executed successfully in simulation mode',
    };
  }

  const startTime = process.hrtime.bigint();

  try {
    const stepHandler = getStepHandler(workflowId, stepName);

    if (!stepHandler) {
      throw new Error(`Step handler not found: ${workflowId}:${stepName}`);
    }

    let contextData: any;
    try {
      contextData = JSON.parse(contextJson);
    } catch (error: any) {
      throw new Error(`Failed to parse context JSON: ${error.message}`);
    }

    if (!contextData || typeof contextData !== 'object') {
      throw new Error('Invalid context: must be an object');
    }

    const requiredFields = ['run_id', 'workflow_id', 'step_name', 'payload'];
    for (const field of requiredFields) {
      if (!contextData[field]) {
        throw new Error(`Missing required context field: ${field}`);
      }
    }

    const contextSize = contextJson.length;
    const maxContextSize = 10 * 1024 * 1024;
    if (contextSize > maxContextSize) {
      throw new Error(
        `Context too large: ${contextSize} bytes (max: ${maxContextSize} bytes)`
      );
    }

    if (contextData.metadata?.checksum) {
      const expectedChecksum = contextData.metadata.checksum;
      const actualChecksum = generateContextChecksum(contextData);
      if (expectedChecksum !== actualChecksum) {
        throw new Error('Context checksum validation failed');
      }
    }

    const currentState = getCurrentState();
    const workflow = currentState.workflows.get(workflowId);

    const services: Record<string, any> = {};

    const enhancedContext = createEnhancedContext(
      contextData,
      workflowId,
      runId
    );

    try {
      const result = await stepHandler.handler(enhancedContext);

      const endTime = process.hrtime.bigint();
      const duration = Number(endTime - startTime) / 1000000;

      return {
        success: true,
        result: {
          step_name: stepName,
          workflow_id: workflowId,
          run_id: runId,
          output: result,
          duration_ms: duration,
          context_size_bytes: contextSize,
          services_count: Object.keys(services).length,
          steps_completed: Object.keys(contextData.steps || {}).length,
          logs: [],
        },
        message: 'Step function executed successfully',
      };
    } catch (error: any) {
      const endTime = process.hrtime.bigint();
      const duration = Number(endTime - startTime) / 1000000;

      return {
        success: false,
        result: {
          step_name: stepName,
          workflow_id: workflowId,
          run_id: runId,
          error: error.message,
          duration_ms: duration,
          context_size_bytes: contextJson.length,
        },
        message: `Step function execution failed: ${error.message}`,
      };
    }
  } catch (error: any) {
    const endTime = process.hrtime.bigint();
    const duration = Number(endTime - startTime) / 1000000;

    return {
      success: false,
      result: {
        step_name: stepName,
        workflow_id: workflowId,
        run_id: runId,
        error: error.message,
        duration_ms: duration,
        context_size_bytes: contextJson.length,
      },
      message: `Step function execution failed: ${error.message}`,
    };
  }
}

export async function executeJobFunction(
  jobJson: string,
  servicesJson: string
): Promise<any> {
  if (!core) {
    return {
      success: true,
      job_id: 'simulation-job-id',
      run_id: 'simulation-run-id',
      step_id: 'simulation-step-id',
      context: servicesJson,
      result: {
        status: 'simulation',
        message: 'Job function executed in simulation mode',
      },
      message: 'Job function executed successfully in simulation mode',
    };
  }

  try {
    const result = core.executeJobFunction(
      jobJson,
      servicesJson,
      getCurrentState().dbPath
    );

    if (result.success && result.result) {
      return JSON.parse(result.result);
    } else {
      throw new Error(`Failed to execute job function: ${result.message}`);
    }
  } catch (error) {
    throw error;
  }
}

function createEnhancedContext(
  contextData: any,
  workflowId: string,
  runId: string
): Context {
  const complexityScore = calculateContextComplexity(contextData);

  const enhancedContext: Context = {
    ...contextData,
    _metadata: {
      ...contextData.metadata,
      complexity_score: complexityScore,
      execution_timestamp: new Date().toISOString(),
      context_version: '1.0.0',
      validation_passed: true,
    },
  };

  return enhancedContext;
}

function calculateContextComplexity(contextData: any): number {
  let score = 1;

  const payloadSize = JSON.stringify(contextData.payload || {}).length;
  if (payloadSize > 100000) score += 2;
  else if (payloadSize > 10000) score += 1;

  const stepsCount = Object.keys(contextData.steps || {}).length;
  if (stepsCount > 50) score += 2;
  else if (stepsCount > 10) score += 1;

  if (hasDeepNesting(contextData.payload, 0)) score += 1;

  return Math.min(score, 10);
}

function hasDeepNesting(obj: any, depth: number): boolean {
  if (depth > 5) return true;
  if (!obj || typeof obj !== 'object') return false;

  if (Array.isArray(obj)) {
    return obj.some(item => hasDeepNesting(item, depth + 1));
  }

  return Object.values(obj).some(value => hasDeepNesting(value, depth + 1));
}

function generateContextChecksum(contextData: any): string {
  const crypto = require('crypto');

  const checksumData = {
    run_id: contextData.run_id,
    workflow_id: contextData.workflow_id,
    step_name: contextData.step_name,
    payload: contextData.payload,
  };

  const checksumString = JSON.stringify(checksumData);
  return crypto.createHash('sha256').update(checksumString).digest('hex');
}

export function createValidContext(
  runId: string,
  workflowId: string,
  stepName: string,
  payload: any = {},
  steps: Record<string, any> = {},
  stepIndex: number = 0,
  totalSteps: number = 1
): string {
  const validRunId = runId.includes('-')
    ? runId
    : `${runId.slice(0, 8)}-${runId.slice(8, 12)}-${runId.slice(12, 16)}-${runId.slice(16, 20)}-${runId.slice(20, 32)}`;

  const context = {
    run_id: validRunId,
    workflow_id: workflowId,
    step_name: stepName,
    payload: payload,
    steps: {},
    run: {
      id: validRunId,
      workflow_id: workflowId,
      status: 'Running',
      payload: payload,
      started_at: new Date().toISOString(),
      completed_at: null,
      error: null,
    },
    metadata: {
      created_at: new Date().toISOString(),
      step_index: stepIndex,
      total_steps: totalSteps,
      timeout: null,
      retry_count: 0,
      max_retries: 3,
    },
  };

  return JSON.stringify(context);
}
