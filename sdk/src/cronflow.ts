import { WorkflowInstance, WorkflowDefinition, Context } from './workflow';
import { createStateManager } from './state';
import * as http from 'http';
import * as url from 'url';
import path from 'path';
import { scheduler } from './scheduler';

// Import the Rust addon
let core: any;
try {
  // Simple and automatic path resolution
  // Get the current file's directory and find the package root
  const currentDir = __dirname;

  // Walk up the directory tree to find where dist/core/core.node exists
  let searchDir = currentDir;
  let corePath = '';

  while (searchDir !== path.dirname(searchDir)) {
    const testPath = path.join(searchDir, 'dist/core/core.node');
    try {
      require.resolve(testPath);
      corePath = testPath;
      break;
    } catch {
      searchDir = path.dirname(searchDir);
    }
  }

  if (corePath) {
    core = require(corePath);
  } else {
    throw new Error('Could not find core.node in dist/core/core.node');
  }
} catch (error) {
  core = null;
}

export const VERSION = '0.1.0';

interface CronflowState {
  workflows: Map<string, WorkflowDefinition>;
  engineState: 'STOPPED' | 'STARTING' | 'STARTED';
  dbPath: string;
  stateManager?: any;
  webhookServer?: http.Server;
  eventListeners: Map<string, Array<{ workflowId: string; trigger: any }>>;
  eventHistory: Array<{ name: string; payload: any; timestamp: number }>;
}

interface StepHandler {
  workflowId: string;
  stepId: string;
  handler: (ctx: Context) => any | Promise<any>;
  type: 'step' | 'action';
}

let stepRegistry: Map<string, StepHandler> = new Map();

let state: CronflowState = {
  workflows: new Map(),
  engineState: 'STOPPED',
  dbPath: './cronflow.db',
  eventListeners: new Map(),
  eventHistory: [],
};

function getCurrentState(): CronflowState {
  return state;
}

function setState(newState: Partial<CronflowState>): void {
  state = { ...state, ...newState };
}

function registerStepHandler(
  workflowId: string,
  stepId: string,
  handler: (ctx: Context) => any | Promise<any>,
  type: 'step' | 'action'
): void {
  const key = `${workflowId}:${stepId}`;
  stepRegistry.set(key, {
    workflowId,
    stepId,
    handler,
    type,
  });
}

function getStepHandler(
  workflowId: string,
  stepId: string
): StepHandler | undefined {
  const key = `${workflowId}:${stepId}`;
  return stepRegistry.get(key);
}

function initialize(dbPath?: string): void {
  setState({
    dbPath: dbPath || './cronflow.db',
    workflows: new Map(),
    engineState: 'STOPPED',
    eventListeners: new Map(),
    eventHistory: [],
  });
}

// State management functions
export async function getGlobalState(
  key: string,
  defaultValue?: any
): Promise<any> {
  const currentState = getCurrentState();
  const stateManager = createStateManager('global', currentState.dbPath);
  return await stateManager.get(key, defaultValue);
}

export async function setGlobalState(
  key: string,
  value: any,
  options?: { ttl?: string }
): Promise<void> {
  const currentState = getCurrentState();
  const stateManager = createStateManager('global', currentState.dbPath);
  await stateManager.set(key, value, options);
}

export async function incrGlobalState(
  key: string,
  amount: number = 1
): Promise<number> {
  const currentState = getCurrentState();
  const stateManager = createStateManager('global', currentState.dbPath);
  return await stateManager.incr(key, amount);
}

export async function deleteGlobalState(key: string): Promise<boolean> {
  const currentState = getCurrentState();
  const stateManager = createStateManager('global', currentState.dbPath);
  return await stateManager.delete(key);
}

export async function getWorkflowState(
  workflowId: string,
  key: string,
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
  options?: { ttl?: string }
): Promise<void> {
  const currentState = getCurrentState();
  const stateManager = createStateManager(workflowId, currentState.dbPath);
  await stateManager.set(key, value, options);
}

export async function incrWorkflowState(
  workflowId: string,
  key: string,
  amount: number = 1
): Promise<number> {
  const currentState = getCurrentState();
  const stateManager = createStateManager(workflowId, currentState.dbPath);
  return await stateManager.incr(key, amount);
}

export async function deleteWorkflowState(
  workflowId: string,
  key: string
): Promise<boolean> {
  const currentState = getCurrentState();
  const stateManager = createStateManager(workflowId, currentState.dbPath);
  return await stateManager.delete(key);
}

export async function getStateStats(): Promise<{
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

export async function cleanupExpiredState(): Promise<{
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

export function define(
  options: Omit<
    WorkflowDefinition,
    'steps' | 'triggers' | 'created_at' | 'updated_at'
  >
): WorkflowInstance {
  if (!options.id || options.id.trim() === '') {
    throw new Error('Workflow ID cannot be empty');
  }

  const currentState = getCurrentState();
  if (currentState.workflows.has(options.id)) {
    throw new Error(`Workflow with ID '${options.id}' already exists`);
  }

  const workflow: WorkflowDefinition = {
    ...options,
    steps: [],
    triggers: [],
    created_at: new Date(),
    updated_at: new Date(),
  };

  const instance = new WorkflowInstance(workflow, cronflow);

  // Register the workflow with the SDK state
  currentState.workflows.set(workflow.id, workflow);

  return instance;
}

export interface WebhookServerConfig {
  host?: string;
  port?: number;
  maxConnections?: number;
}

export interface StartOptions {
  webhookServer?: WebhookServerConfig;
}

export async function start(options?: StartOptions): Promise<void> {
  const currentState = getCurrentState();

  if (
    currentState.engineState === 'STARTED' ||
    currentState.engineState === 'STARTING'
  ) {
    return Promise.resolve();
  }

  setState({ engineState: 'STARTING' });

  // Start the Node.js scheduler (only if not already running)
  if (!scheduler.getRunningStatus()) {
    scheduler.start();
  } else {
  }

  if (!currentState.dbPath) {
    const defaultDbPath = './cronflow.db';
    setState({ dbPath: defaultDbPath });
  }

  if (core) {
    try {
      if (options?.webhookServer) {
        const webhookConfig = {
          host: options.webhookServer.host || '127.0.0.1',
          port: options.webhookServer.port || 3000,
          max_connections: options.webhookServer.maxConnections || 1000,
        };

        // TODO: Expose webhook server configuration through N-API
        // For now, we'll use environment variables as a workaround
        if (webhookConfig.host !== '127.0.0.1') {
          process.env.CRONFLOW_WEBHOOK_HOST = webhookConfig.host;
        }
        if (webhookConfig.port !== 3000) {
          process.env.CRONFLOW_WEBHOOK_PORT = webhookConfig.port.toString();
        }
        if (webhookConfig.max_connections !== 1000) {
          process.env.CRONFLOW_WEBHOOK_MAX_CONNECTIONS =
            webhookConfig.max_connections.toString();
        }
      }

      for (const workflow of currentState.workflows.values()) {
        await registerWorkflowWithRust(workflow);
      }

      if (options?.webhookServer) {
        const result = core.startWebhookServer(currentState.dbPath);
        if (!result.success) {
          throw new Error(`Failed to start webhook server: ${result.message}`);
        }

        const server = http.createServer(async (req, res) => {
          try {
            const parsedUrl = url.parse(req.url || '', true);
            const path = parsedUrl.pathname || '';

            if (path === '/health') {
              res.writeHead(200, { 'Content-Type': 'application/json' });
              res.end(
                JSON.stringify({
                  status: 'healthy',
                  service: 'node-cronflow-webhook-server',
                  timestamp: new Date().toISOString(),
                })
              );
              return;
            }

            if (path.startsWith('/webhook/')) {
              const webhookPath = path.replace('/webhook', '');

              const workflow = Array.from(currentState.workflows.values()).find(
                w =>
                  w.triggers.some(
                    t => t.type === 'webhook' && t.path === webhookPath
                  )
              );

              if (!workflow) {
                res.writeHead(404, { 'Content-Type': 'application/json' });
                res.end(JSON.stringify({ error: 'Webhook not found' }));
                return;
              }

              const webhookTrigger = workflow.triggers.find(
                t => t.type === 'webhook' && t.path === webhookPath
              ) as { type: 'webhook'; path: string; options?: any } | undefined;

              const headers: Record<string, string> = {};
              for (const [key, value] of Object.entries(req.headers)) {
                if (Array.isArray(value)) {
                  headers[key.toLowerCase()] = value[0];
                } else if (value) {
                  headers[key.toLowerCase()] = value;
                }
              }

              if (webhookTrigger?.options?.headers) {
                const headerConfig = webhookTrigger.options.headers;

                if (headerConfig.required) {
                  for (const [requiredHeader, expectedValue] of Object.entries(
                    headerConfig.required
                  )) {
                    const actualValue = headers[requiredHeader.toLowerCase()];
                    if (!actualValue) {
                      res.writeHead(400, {
                        'Content-Type': 'application/json',
                      });
                      res.end(
                        JSON.stringify({
                          error: `Missing required header: ${requiredHeader}`,
                          required_headers: headerConfig.required,
                        })
                      );
                      return;
                    }
                    if (expectedValue && actualValue !== expectedValue) {
                      res.writeHead(400, {
                        'Content-Type': 'application/json',
                      });
                      res.end(
                        JSON.stringify({
                          error: `Invalid header value for ${requiredHeader}: expected ${expectedValue}, got ${actualValue}`,
                          required_headers: headerConfig.required,
                        })
                      );
                      return;
                    }
                  }
                }

                if (headerConfig.validate) {
                  const validationResult = headerConfig.validate(headers);
                  if (validationResult !== true) {
                    const errorMessage =
                      typeof validationResult === 'string'
                        ? validationResult
                        : 'Header validation failed';
                    res.writeHead(400, { 'Content-Type': 'application/json' });
                    res.end(JSON.stringify({ error: errorMessage }));
                    return;
                  }
                }
              }

              let body = '';
              req.on('data', chunk => {
                body += chunk.toString();
              });

              req.on('end', async () => {
                try {
                  const payload = body ? JSON.parse(body) : {};

                  if (webhookTrigger?.options?.schema) {
                    try {
                      webhookTrigger.options.schema.parse(payload);
                    } catch (schemaError: any) {
                      res.writeHead(400, {
                        'Content-Type': 'application/json',
                      });
                      res.end(
                        JSON.stringify({
                          status: 'error',
                          message: 'Payload validation failed',
                          error: schemaError.message,
                          workflow_triggered: false,
                        })
                      );
                      return;
                    }
                  }

                  const runId = await trigger(workflow.id, payload);

                  res.writeHead(200, { 'Content-Type': 'application/json' });
                  res.end(
                    JSON.stringify({
                      status: 'success',
                      message: 'Webhook processed successfully',
                      workflow_triggered: true,
                      run_id: runId,
                    })
                  );
                } catch (error) {
                  res.writeHead(500, { 'Content-Type': 'application/json' });
                  res.end(
                    JSON.stringify({
                      status: 'error',
                      message:
                        error instanceof Error
                          ? error.message
                          : 'Unknown error',
                      workflow_triggered: false,
                    })
                  );
                }
              });

              return;
            }

            res.writeHead(404, { 'Content-Type': 'application/json' });
            res.end(JSON.stringify({ error: 'Not found' }));
          } catch (error) {
            res.writeHead(500, { 'Content-Type': 'application/json' });
            res.end(JSON.stringify({ error: 'Internal server error' }));
          }
        });

        const host = options.webhookServer.host || '127.0.0.1';
        const port = options.webhookServer.port || 3000;

        server.listen(port, host, () => {
          setState({ webhookServer: server });
        });

        setState({ webhookServer: server });
      }
    } catch (error) {
      throw error;
    }
  } else {
    // Running in simulation mode
  }

  setState({ engineState: 'STARTED' });
}

export async function stop(): Promise<void> {
  const currentState = getCurrentState();

  // Stop the Node.js scheduler
  scheduler.stop();

  if (currentState.webhookServer) {
    currentState.webhookServer.close(() => {
      // Webhook server closed
    });

    setState({ webhookServer: undefined });
  }

  setState({ engineState: 'STOPPED' });
}

export async function trigger(
  workflowId: string,
  payload: any
): Promise<string> {
  const currentState = getCurrentState();

  if (!core) {
    return 'simulation-run-id';
  }

  try {
    const payloadJson = JSON.stringify(payload);
    const result = core.createRun(workflowId, payloadJson, currentState.dbPath);

    if (result.success && result.runId) {
      await executeWorkflowSteps(workflowId, result.runId, payload);

      return result.runId;
    } else if (result.success) {
      // Fallback to generating a run ID if none was returned
      const fallbackRunId = `run_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;

      await executeWorkflowSteps(workflowId, fallbackRunId, payload);

      return fallbackRunId;
    } else {
      throw new Error(`Failed to trigger workflow: ${result.message}`);
    }
  } catch (error) {
    throw error;
  }
}

async function executeWorkflowSteps(
  workflowId: string,
  runId: string,
  payload: any
): Promise<void> {
  const currentState = getCurrentState();

  if (!core) {
    return;
  }

  try {
    // Get the workflow definition
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
                  ).catch(hookError => {});
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
                    background: true,
                  };
                  onFailureHandler(
                    'onFailure',
                    JSON.stringify(stepContext),
                    workflowId,
                    step.name
                  ).catch(hookError => {});
                } catch (hookError) {}
              }
            }
          })
          .catch(error => {
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
                ).catch(hookError => {});
              } catch (hookError) {}
            }
          });

        // For background steps/actions, we don't wait for completion and don't store results
        // The workflow continues immediately to the next step
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
        lastExecutedStepIndex = i; // Update the last executed step index

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

export async function inspect(runId: string): Promise<any> {
  const currentState = getCurrentState();

  if (!core) {
    return {
      id: runId,
      status: 'simulation',
      workflow_id: 'simulation-workflow',
      created_at: new Date().toISOString(),
    };
  }

  try {
    const result = core.getRunStatus(runId, currentState.dbPath);

    if (result.success && result.status) {
      return JSON.parse(result.status);
    } else {
      throw new Error(`Failed to inspect run: ${result.message}`);
    }
  } catch (error) {
    throw error;
  }
}

export async function cancelRun(runId: string): Promise<void> {
  // TODO: Implement run cancellation
}

export async function publishEvent(name: string, payload: any): Promise<void> {
  const currentState = getCurrentState();

  currentState.eventHistory.push({
    name,
    payload,
    timestamp: Date.now(),
  });

  if (currentState.eventHistory.length > 1000) {
    currentState.eventHistory = currentState.eventHistory.slice(-1000);
  }

  const listeners = currentState.eventListeners.get(name) || [];

  if (listeners.length === 0) {
    return;
  }

  const triggerPromises = listeners.map(async listener => {
    try {
      const { workflowId, trigger } = listener;

      const eventPayload = {
        event: {
          name,
          payload,
          timestamp: Date.now(),
        },
        ...payload,
      };

      const runId = await trigger(workflowId, eventPayload);

      return { workflowId, runId, success: true };
    } catch (error) {
      return { workflowId: listener.workflowId, error, success: false };
    }
  });

  const results = await Promise.allSettled(triggerPromises);

  const successful = results.filter(
    r => r.status === 'fulfilled' && r.value.success
  ).length;
  const failed = results.length - successful;

  results.forEach((result, index) => {
    if (result.status === 'rejected') {
    } else if (result.status === 'fulfilled' && !result.value.success) {
    }
  });
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
      '{}' // Empty services JSON
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
    const maxContextSize = 10 * 1024 * 1024; // 10MB limit
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

    // Create empty services object since services are no longer supported
    const services: Record<string, any> = {};

    const enhancedContext = createEnhancedContext(
      contextData,
      workflowId,
      runId
    );

    try {
      const result = await stepHandler.handler(enhancedContext);

      const endTime = process.hrtime.bigint();
      const duration = Number(endTime - startTime) / 1000000; // Convert to milliseconds

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

export function getWorkflows(): WorkflowDefinition[] {
  const currentState = getCurrentState();
  return Array.from(currentState.workflows.values());
}

export function getWorkflow(id: string): WorkflowDefinition | undefined {
  const currentState = getCurrentState();
  return currentState.workflows.get(id);
}

async function registerWorkflowWithRust(
  workflow: WorkflowDefinition
): Promise<void> {
  if (!core) {
    return;
  }

  const currentState = getCurrentState();

  try {
    const rustFormat = convertToRustFormat(workflow);
    const workflowJson = JSON.stringify(rustFormat);

    const result = core.registerWorkflow(workflowJson, currentState.dbPath);

    if (!result.success) {
      throw new Error(`Failed to register workflow: ${result.message}`);
    }
  } catch (error) {
    throw error;
  }
}

function convertToRustFormat(workflow: WorkflowDefinition): any {
  return {
    id: workflow.id,
    name: workflow.name || workflow.id,
    description: workflow.description || '',
    steps: workflow.steps.map(step => ({
      id: step.id,
      name: step.name,
      title: step.title,
      description: step.description,
      action: step.handler.toString(),
      type: step.type,
      handler: step.handler.toString(),
      timeout: step.options?.timeout || 30000,
      retry: step.options?.retry
        ? {
            max_attempts: step.options.retry.attempts,
            backoff_ms: parseDuration(step.options.retry.backoff.delay),
          }
        : { max_attempts: 1, backoff_ms: 1000 },
      depends_on: [],
      is_control_flow: false,
      condition_type: null,
      condition_expression: null,
      control_flow_block: null,
      parallel: null,
      parallel_group_id: null,
      parallel_step_count: null,
      race: null,
      for_each: null,
      pause: null,
    })),
    triggers: workflow.triggers.map(trigger => {
      if (trigger.type === 'webhook') {
        return {
          Webhook: {
            path: trigger.path,
            method: trigger.options?.method || 'POST',
          },
        };
      } else if (trigger.type === 'schedule') {
        return 'Manual';
      } else if (trigger.type === 'event') {
        return 'Manual';
      } else {
        return 'Manual';
      }
    }),
    created_at: workflow.created_at.toISOString(),
    updated_at: workflow.updated_at.toISOString(),
  };
}

function parseDuration(duration: string | number): number {
  if (typeof duration === 'number') {
    return duration;
  }

  const match = duration.match(/^(\d+)(ms|s|m|h|d)$/);
  if (!match) {
    throw new Error(`Invalid duration format: ${duration}`);
  }

  const [, value, unit] = match;
  const numValue = parseInt(value, 10);

  switch (unit) {
    case 'ms':
      return numValue;
    case 's':
      return numValue * 1000;
    case 'm':
      return numValue * 60 * 1000;
    case 'h':
      return numValue * 60 * 60 * 1000;
    case 'd':
      return numValue * 24 * 60 * 60 * 1000;
    default:
      throw new Error(`Unknown duration unit: ${unit}`);
  }
}

export function getEngineState(): 'STOPPED' | 'STARTING' | 'STARTED' {
  return getCurrentState().engineState;
}

export async function replay(
  runId: string,
  options?: {
    overridePayload?: any;
    mockStep?: (stepName: string, mockFn: (ctx: Context) => any) => void;
  }
): Promise<void> {
  // TODO: Implement replay functionality
}

const pausedWorkflows = new Map<
  string,
  {
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
>();

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

export function getPausedWorkflow(token: string) {
  return pausedWorkflows.get(token);
}

export function listPausedWorkflows() {
  return Array.from(pausedWorkflows.values());
}

export function isRustCoreAvailable(): boolean {
  return core !== null;
}

function getMemoryUsage() {
  const usage = process.memoryUsage();
  return {
    rss: Math.round(usage.rss / 1024 / 1024),
    heapTotal: Math.round(usage.heapTotal / 1024 / 1024),
    heapUsed: Math.round(usage.heapUsed / 1024 / 1024),
    external: Math.round(usage.external / 1024 / 1024),
  };
}

function formatDuration(ms: number): string {
  if (ms < 1) return `${(ms * 1000).toFixed(2)}μs`;
  if (ms < 1000) return `${ms.toFixed(2)}ms`;
  if (ms < 60000) return `${(ms / 1000).toFixed(2)}s`;
  return `${(ms / 60000).toFixed(2)}m`;
}

function calculateStats(values: number[]) {
  const sorted = values.sort((a, b) => a - b);
  const sum = values.reduce((a, b) => a + b, 0);
  const mean = sum / values.length;
  const median = sorted[Math.floor(sorted.length / 2)];
  const min = sorted[0];
  const max = sorted[sorted.length - 1];
  const variance =
    values.reduce((acc, val) => acc + Math.pow(val - mean, 2), 0) /
    values.length;
  const stdDev = Math.sqrt(variance);

  return {
    count: values.length,
    mean,
    median,
    min,
    max,
    stdDev,
    p95: sorted[Math.floor(sorted.length * 0.95)],
    p99: sorted[Math.floor(sorted.length * 0.99)],
  };
}

export interface BenchmarkOptions {
  iterations?: number;
  stepsPerWorkflow?: number;
  payloadSize?: number;
  delayBetweenRuns?: number;
  verbose?: boolean;
}

export interface BenchmarkResult {
  success: boolean;
  statistics: {
    duration: {
      count: number;
      mean: number;
      median: number;
      min: number;
      max: number;
      stdDev: number;
      p95: number;
      p99: number;
    };
    memory: {
      count: number;
      mean: number;
      median: number;
      min: number;
      max: number;
      stdDev: number;
      p95: number;
      p99: number;
    };
    successRate: number;
    throughput: number;
    stepsPerSecond: number;
    averageStepTime: number;
  };
  results: Array<{
    iteration: number;
    success: boolean;
    runId?: string;
    duration: number;
    error?: string;
    memory?: {
      start: any;
      end: any;
      delta: {
        rss: number;
        heapUsed: number;
      };
    };
  }>;
}

export async function benchmark(
  options: BenchmarkOptions = {}
): Promise<BenchmarkResult> {
  const {
    iterations = 10,
    stepsPerWorkflow = 3,
    payloadSize = 100,
    delayBetweenRuns = 100,
    verbose = true,
  } = options;

  const results: any[] = [];
  const durations: number[] = [];
  const memoryDeltas: number[] = [];
  const benchmarkId = Date.now().toString();

  for (let i = 1; i <= iterations; i++) {
    const startTime = process.hrtime.bigint();
    const startMemory = getMemoryUsage();

    try {
      const workflow = define({
        id: `benchmark-${benchmarkId}-${i}`,
        name: `Benchmark Workflow ${i}`,
        description: 'Performance test workflow',
      });

      for (let stepNum = 1; stepNum <= stepsPerWorkflow; stepNum++) {
        workflow.step(`step${stepNum}`, async (ctx: Context) => {
          return {
            message: `Step ${stepNum} completed`,
            previous: ctx.last,
            timestamp: Date.now(),
            stepNumber: stepNum,
          };
        });
      }

      await start();

      const payload = {
        iteration: i,
        timestamp: Date.now(),
        data: Array.from({ length: payloadSize }, (_, j) => `item-${j}`),
      };

      const runId = await trigger(`benchmark-${benchmarkId}-${i}`, payload);

      const endTime = process.hrtime.bigint();
      const endMemory = getMemoryUsage();
      const duration = Number(endTime - startTime) / 1000000;

      const result = {
        iteration: i,
        success: true,
        runId,
        duration,
        memory: {
          start: startMemory,
          end: endMemory,
          delta: {
            rss: endMemory.rss - startMemory.rss,
            heapUsed: endMemory.heapUsed - startMemory.heapUsed,
          },
        },
      };

      results.push(result);
      durations.push(duration);
      memoryDeltas.push(result.memory.delta.heapUsed);
    } catch (error) {
      const endTime = process.hrtime.bigint();
      const duration = Number(endTime - startTime) / 1000000;

      const result = {
        iteration: i,
        success: false,
        error: error instanceof Error ? error.message : String(error),
        duration,
      };

      results.push(result);
    } finally {
      await stop();
    }

    if (delayBetweenRuns > 0) {
      await new Promise(resolve => setTimeout(resolve, delayBetweenRuns));
    }
  }

  const successfulRuns = results.filter(r => r.success).length;
  const successRate = (successfulRuns / iterations) * 100;

  let durationStats, memoryStats, stepsPerSecond, averageStepTime, throughput;

  if (successfulRuns > 0) {
    durationStats = calculateStats(durations);
    memoryStats = calculateStats(memoryDeltas);
    const totalSteps = successfulRuns * stepsPerWorkflow;
    stepsPerSecond = totalSteps / (durationStats.mean / 1000);
    averageStepTime = durationStats.mean / stepsPerWorkflow;
    throughput = iterations / (durationStats.mean / 1000);
  } else {
    durationStats = {
      count: 0,
      mean: 0,
      median: 0,
      min: 0,
      max: 0,
      stdDev: 0,
      p95: 0,
      p99: 0,
    };
    memoryStats = {
      count: 0,
      mean: 0,
      median: 0,
      min: 0,
      max: 0,
      stdDev: 0,
      p95: 0,
      p99: 0,
    };
    stepsPerSecond = 0;
    averageStepTime = 0;
    throughput = 0;
  }

  return {
    success: successfulRuns > 0,
    statistics: {
      duration: durationStats,
      memory: memoryStats,
      successRate,
      throughput,
      stepsPerSecond,
      averageStepTime,
    },
    results,
  };
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
      getCurrentState().dbPath
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
      getCurrentState().dbPath
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
      getCurrentState().dbPath
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
    const result = core.getTriggerStats(getCurrentState().dbPath);

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
      getCurrentState().dbPath
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
      getCurrentState().dbPath
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
    const result = core.getScheduleTriggers(getCurrentState().dbPath);

    if (result.success) {
      return result;
    } else {
      throw new Error(`Failed to get schedule triggers: ${result.message}`);
    }
  } catch (error) {
    throw error;
  }
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

initialize();

const hookHandlers = new Map<
  string,
  (
    hookType: string,
    contextJson: string,
    workflowId: string,
    stepId?: string
  ) => Promise<any>
>();

function registerHookHandler(
  hookType: 'onSuccess' | 'onFailure',
  handler: (
    hookType: string,
    contextJson: string,
    workflowId: string,
    stepId?: string
  ) => Promise<any>
): void {
  hookHandlers.set(hookType, handler);
}

registerHookHandler('onSuccess', executeWorkflowHook);
registerHookHandler('onFailure', executeWorkflowHook);

function getHookHandler(
  hookType: string
):
  | ((
      hookType: string,
      contextJson: string,
      workflowId: string,
      stepId?: string
    ) => Promise<any>)
  | undefined {
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

    const workflow = getWorkflow(workflowId);
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

function registerEventListener(
  eventName: string,
  workflowId: string,
  trigger: any
): void {
  const currentState = getCurrentState();

  if (!currentState.eventListeners.has(eventName)) {
    currentState.eventListeners.set(eventName, []);
  }

  const listeners = currentState.eventListeners.get(eventName)!;

  const existingIndex = listeners.findIndex(l => l.workflowId === workflowId);
  if (existingIndex >= 0) {
    listeners[existingIndex] = { workflowId, trigger };
  } else {
    listeners.push({ workflowId, trigger });
  }
}

function unregisterEventListener(eventName: string, workflowId: string): void {
  const currentState = getCurrentState();

  const listeners = currentState.eventListeners.get(eventName);
  if (!listeners) {
    return;
  }

  const index = listeners.findIndex(l => l.workflowId === workflowId);
  if (index >= 0) {
    listeners.splice(index, 1);

    if (listeners.length === 0) {
      currentState.eventListeners.delete(eventName);
    }
  }
}

function getEventHistory(
  eventName?: string
): Array<{ name: string; payload: any; timestamp: number }> {
  const currentState = getCurrentState();

  if (eventName) {
    return currentState.eventHistory.filter(event => event.name === eventName);
  }

  return [...currentState.eventHistory];
}

function getEventListeners(
  eventName?: string
): Map<string, Array<{ workflowId: string; trigger: any }>> {
  const currentState = getCurrentState();

  if (eventName) {
    const listeners = currentState.eventListeners.get(eventName);
    const result = new Map();
    if (listeners) {
      result.set(eventName, listeners);
    }
    return result;
  }

  return new Map(currentState.eventListeners);
}

export {
  registerEventListener,
  unregisterEventListener,
  getEventHistory,
  getEventListeners,
};

export const cronflow = {
  define,
  start,
  stop,
  trigger,
  inspect,
  cancelRun,
  publishEvent,
  executeStep,
  executeStepFunction,
  executeJobFunction,
  getWorkflows,
  getWorkflow,
  getEngineState,
  replay,
  resume,
  isRustCoreAvailable,
  benchmark,
  executeManualTrigger,
  executeWebhookTrigger,
  executeScheduleTrigger,
  getTriggerStats,
  getWorkflowTriggers,
  unregisterWorkflowTriggers,
  getScheduleTriggers,
  createValidContext,
  executeWorkflowHook,
  getGlobalState,
  setGlobalState,
  incrGlobalState,
  deleteGlobalState,
  getWorkflowState,
  setWorkflowState,
  incrWorkflowState,
  deleteWorkflowState,
  getStateStats,
  cleanupExpiredState,
  registerStepHandler,
  VERSION,
  registerEventListener,
  unregisterEventListener,
  getEventHistory,
  getEventListeners,
  scheduler,
};
