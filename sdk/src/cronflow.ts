import { WorkflowInstance, WorkflowDefinition, Context } from './workflow';
import { createStateManager } from './state';
import * as http from 'http';
import * as url from 'url';

// Import the Rust addon
let core: any;
try {
  core = require('../../core/core.node');
} catch (error) {
  console.warn('⚠️  Rust core not available, running in simulation mode');
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
  console.log(`📝 Registered step handler: ${key} (${type})`);
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
  console.log(`Node-Cronflow SDK v${VERSION} initialized`);
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
  const currentState = getCurrentState();

  if (!options.id || options.id.trim() === '') {
    throw new Error('Workflow ID cannot be empty');
  }

  if (currentState.workflows.has(options.id)) {
    throw new Error(`Workflow with ID '${options.id}' already exists`);
  }

  if (options.timeout !== undefined) {
    if (typeof options.timeout === 'string') {
      const timeoutRegex = /^(\d+)(s|m|h|d)$/;
      if (!timeoutRegex.test(options.timeout)) {
        throw new Error(
          'Invalid timeout format. Use format like "5m", "1h", "30s"'
        );
      }
    } else if (typeof options.timeout === 'number') {
      if (options.timeout <= 0) {
        throw new Error('Timeout must be a positive number');
      }
    } else {
      throw new Error('Timeout must be a string or number');
    }
  }

  if (options.concurrency !== undefined) {
    if (!Number.isInteger(options.concurrency) || options.concurrency <= 0) {
      throw new Error('Concurrency must be a positive integer');
    }
  }

  if (options.rateLimit) {
    if (
      !options.rateLimit.count ||
      !Number.isInteger(options.rateLimit.count) ||
      options.rateLimit.count <= 0
    ) {
      throw new Error('Rate limit count must be a positive integer');
    }

    if (!options.rateLimit.per || typeof options.rateLimit.per !== 'string') {
      throw new Error('Rate limit per must be a string (e.g., "1m", "1h")');
    }

    const rateLimitRegex = /^(\d+)(s|m|h|d)$/;
    if (!rateLimitRegex.test(options.rateLimit.per)) {
      throw new Error(
        'Invalid rate limit time format. Use format like "1m", "1h"'
      );
    }
  }

  if (options.queue !== undefined) {
    if (typeof options.queue !== 'string' || options.queue.trim() === '') {
      throw new Error('Queue must be a non-empty string');
    }
  }

  if (options.version !== undefined) {
    if (typeof options.version !== 'string' || options.version.trim() === '') {
      throw new Error('Version must be a non-empty string');
    }

    const versionRegex =
      /^v?\d+\.\d+\.\d+(-[a-zA-Z0-9.-]+)?(\+[a-zA-Z0-9.-]+)?$/;
    if (!versionRegex.test(options.version)) {
      console.warn(
        `Warning: Version '${options.version}' may not follow semantic versioning format`
      );
    }
  }

  if (options.tags) {
    if (!Array.isArray(options.tags)) {
      throw new Error('Tags must be an array');
    }
    for (const tag of options.tags) {
      if (typeof tag !== 'string' || tag.trim() === '') {
        throw new Error('All tags must be non-empty strings');
      }
    }
  }

  if (options.services) {
    if (!Array.isArray(options.services)) {
      throw new Error('Services must be an array');
    }
    for (const service of options.services) {
      if (!service || typeof service !== 'object') {
        throw new Error('Invalid service provided to workflow definition');
      }

      if (!service.id || typeof service.id !== 'string') {
        throw new Error('Service must have a valid id property');
      }

      if (!service.actions || typeof service.actions !== 'object') {
        throw new Error('Service must have actions property');
      }

      for (const [actionName, actionFn] of Object.entries(service.actions)) {
        if (typeof actionFn !== 'function') {
          throw new Error(`Service action '${actionName}' must be a function`);
        }
      }
    }
  }

  if (options.hooks) {
    if (typeof options.hooks !== 'object') {
      throw new Error('Hooks must be an object');
    }

    if (
      options.hooks.onSuccess !== undefined &&
      typeof options.hooks.onSuccess !== 'function'
    ) {
      throw new Error('onSuccess hook must be a function');
    }

    if (
      options.hooks.onFailure !== undefined &&
      typeof options.hooks.onFailure !== 'function'
    ) {
      throw new Error('onFailure hook must be a function');
    }
  }

  if (options.secrets !== undefined) {
    if (typeof options.secrets !== 'object' || options.secrets === null) {
      throw new Error('Secrets must be an object');
    }
  }

  const workflow: WorkflowDefinition = {
    ...options,
    steps: [],
    triggers: [],
    created_at: new Date(),
    updated_at: new Date(),
  };

  currentState.workflows.set(options.id, workflow);
  console.log(`Workflow '${options.id}' defined successfully`);

  return new WorkflowInstance(workflow, {
    define,
    start,
    stop,
    trigger,
    inspect,
    registerWorkflow: async (workflow: WorkflowDefinition) => {
      const currentState = getCurrentState();
      currentState.workflows.set(workflow.id, workflow);
      console.log(`Workflow '${workflow.id}' registered with SDK state`);
    },
    registerStepHandler: (
      workflowId: string,
      stepId: string,
      handler: (ctx: Context) => any | Promise<any>,
      type: 'step' | 'action'
    ) => {
      registerStepHandler(workflowId, stepId, handler, type);
    },
  });
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
  console.log('Starting Node-Cronflow engine...');

  if (!currentState.dbPath) {
    const defaultDbPath = './cronflow.db';
    setState({ dbPath: defaultDbPath });
    console.log(`🔧 Using default database path: ${defaultDbPath}`);
  }

  if (core) {
    try {
      if (options?.webhookServer) {
        const webhookConfig = {
          host: options.webhookServer.host || '127.0.0.1',
          port: options.webhookServer.port || 3000,
          max_connections: options.webhookServer.maxConnections || 1000,
        };

        console.log(
          `🔧 Configuring webhook server: ${webhookConfig.host}:${webhookConfig.port}`
        );

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
      console.log('✅ All workflows registered with Rust engine');

      if (options?.webhookServer) {
        const result = core.startWebhookServer(currentState.dbPath);
        if (!result.success) {
          throw new Error(`Failed to start webhook server: ${result.message}`);
        }
        console.log('✅ Webhook server started successfully');

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
                      console.error('Schema validation failed:', schemaError);
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
                  console.error('Webhook processing error:', error);
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
            console.error('HTTP server error:', error);
            res.writeHead(500, { 'Content-Type': 'application/json' });
            res.end(JSON.stringify({ error: 'Internal server error' }));
          }
        });

        const host = options.webhookServer.host || '127.0.0.1';
        const port = options.webhookServer.port || 3000;

        server.listen(port, host, () => {
          console.log(`🌐 Webhook server running on: http://${host}:${port}`);
          console.log(
            `📡 Webhook endpoint: http://${host}:${port}/webhook/webhooks/simple`
          );
          console.log(`🏥 Health check: http://${host}:${port}/health`);
        });

        setState({ webhookServer: server });
      }
    } catch (error) {
      console.error('❌ Failed to register workflows with Rust engine:', error);
      throw error;
    }
  } else {
    console.log('⚠️  Running in simulation mode (Rust core not available)');
  }

  setState({ engineState: 'STARTED' });
  console.log('Node-Cronflow engine started successfully');
}

export async function stop(): Promise<void> {
  const currentState = getCurrentState();

  if (currentState.webhookServer) {
    currentState.webhookServer.close(() => {
      console.log('✅ Webhook HTTP server closed successfully');
    });
  }

  if (core) {
    try {
      const result = core.stopWebhookServer(currentState.dbPath);
      if (!result.success) {
        console.warn(`⚠️  Failed to stop webhook server: ${result.message}`);
      } else {
        console.log('✅ Webhook server stopped successfully');
      }
    } catch (error) {
      console.warn('⚠️  Error stopping webhook server:', error);
    }
  }

  setState({ engineState: 'STOPPED', webhookServer: undefined });
  console.log('Node-Cronflow engine stopped');
}

export async function trigger(
  workflowId: string,
  payload: any
): Promise<string> {
  const currentState = getCurrentState();

  if (!core) {
    console.log(
      `⚠️  Simulation: Triggering workflow: ${workflowId} with payload:`,
      payload
    );
    return 'simulation-run-id';
  }

  try {
    const payloadJson = JSON.stringify(payload);
    const result = core.createRun(workflowId, payloadJson, currentState.dbPath);

    if (result.success && result.runId) {
      console.log(
        `✅ Workflow triggered successfully: ${workflowId} -> ${result.runId}`
      );

      await executeWorkflowSteps(workflowId, result.runId, payload);

      return result.runId;
    } else if (result.success) {
      console.log(
        `⚠️  Workflow triggered but no runId returned: ${workflowId} -> ${result.message}`
      );
      // Fallback to generating a run ID if none was returned
      const fallbackRunId = `run_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;

      await executeWorkflowSteps(workflowId, fallbackRunId, payload);

      return fallbackRunId;
    } else {
      throw new Error(`Failed to trigger workflow: ${result.message}`);
    }
  } catch (error) {
    console.error(`❌ Failed to trigger workflow ${workflowId}:`, error);
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
    console.log(
      `⚠️  Simulation: Executing workflow steps for ${workflowId} run ${runId}`
    );
    return;
  }

  try {
    // Get the workflow definition
    const workflow = currentState.workflows.get(workflowId);
    if (!workflow) {
      throw new Error(`Workflow ${workflowId} not found`);
    }

    console.log(`🔄 Executing workflow steps for ${workflowId} run ${runId}`);
    console.log(`📋 Workflow has ${workflow.steps.length} steps`);

    const completedSteps: Record<string, any> = {};
    let currentConditionMet = false;
    let inControlFlowBlock = false;
    let skipUntilEndIf = false;
    let lastExecutedStepIndex = -1;

    for (let i = 0; i < workflow.steps.length; i++) {
      const step = workflow.steps[i];
      console.log(
        `\n📝 Executing step ${i + 1}/${workflow.steps.length}: ${step.name}`
      );

      const isControlFlowStep =
        step.options?.conditionType ||
        step.name.startsWith('conditional_') ||
        step.name.startsWith('else_') ||
        step.name.startsWith('endif_');

      if (isControlFlowStep) {
        console.log(`🔀 Control flow step detected: ${step.name}`);

        if (
          step.name.startsWith('conditional_') ||
          step.options?.conditionType === 'if'
        ) {
          console.log(`🔍 Evaluating IF condition: ${step.name}`);

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
            console.log(`✅ IF condition result: ${currentConditionMet}`);

            if (currentConditionMet) {
              inControlFlowBlock = true;
              skipUntilEndIf = false;
            } else {
              skipUntilEndIf = true;
            }
          } else {
            console.log(
              `⚠️  No handler found for condition step: ${step.name}`
            );
            skipUntilEndIf = true;
          }

          console.log(`⏭️  Skipping control flow step execution: ${step.name}`);
          continue;
        } else if (
          step.name.startsWith('else_') ||
          step.options?.conditionType === 'else'
        ) {
          console.log(`🔀 Processing ELSE condition: ${step.name}`);

          if (!currentConditionMet && !skipUntilEndIf) {
            inControlFlowBlock = true;
            skipUntilEndIf = false;
            console.log(`✅ ELSE branch will be executed`);
          } else {
            skipUntilEndIf = true;
            console.log(`⏭️  ELSE branch will be skipped`);
          }

          console.log(`⏭️  Skipping control flow step execution: ${step.name}`);
          continue;
        } else if (step.name.startsWith('endif_')) {
          console.log(`🔚 Processing ENDIF: ${step.name}`);

          inControlFlowBlock = false;
          skipUntilEndIf = false;
          currentConditionMet = false;

          console.log(`⏭️  Skipping control flow step execution: ${step.name}`);
          continue;
        }
      }

      if (skipUntilEndIf) {
        console.log(`⏭️  Skipping step due to control flow: ${step.name}`);
        continue;
      }

      if (step.options?.pause || step.name === 'pause') {
        console.log(`⏸️  Pause step detected: ${step.name}`);

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
          console.log(`✅ Pause step ${step.name} completed successfully`);

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

          console.log(`⏸️  Workflow paused at step: ${step.name}`);
          console.log(`🔑 Pause token: ${pauseInfo.token}`);
          console.log(
            `🔄 Use cronflow.resume('${pauseInfo.token}', payload) to resume`
          );

          return;
        } else {
          console.error(
            `❌ Pause step ${step.name} failed:`,
            stepResult.message
          );
          throw new Error(
            `Pause step ${step.name} failed: ${stepResult.message}`
          );
        }
      }

      if (step.options?.parallel || step.parallel) {
        console.log(`🔄 Executing parallel step: ${step.name}`);

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
          console.log(`✅ Parallel step ${step.name} completed successfully`);
        } else {
          console.error(
            `❌ Parallel step ${step.name} failed:`,
            stepResult.message
          );
          throw new Error(
            `Parallel step ${step.name} failed: ${stepResult.message}`
          );
        }
        continue;
      }

      console.log(`🔄 Executing regular step: ${step.name}`);

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
        console.log(
          `🔄 Executing ${step.type === 'action' ? 'action' : 'step'} as background side effect: ${step.name}`
        );

        executeStepFunction(step.name, contextJson, workflowId, runId)
          .then(stepResult => {
            if (stepResult.success && stepResult.result) {
              console.log(
                `✅ ${step.type === 'action' ? 'Action' : 'Step'} ${step.name} completed successfully in background`
              );

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
                  ).catch(hookError => {
                    console.warn(
                      `⚠️  Background step-level onSuccess hook failed for ${step.name}:`,
                      hookError
                    );
                  });
                } catch (hookError) {
                  console.warn(
                    `⚠️  Background step-level onSuccess hook failed for ${step.name}:`,
                    hookError
                  );
                }
              }
            } else {
              console.error(
                `❌ ${step.type === 'action' ? 'Action' : 'Step'} ${step.name} failed in background:`,
                stepResult.message
              );

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
                  ).catch(hookError => {
                    console.warn(
                      `⚠️  Background step-level onFailure hook failed for ${step.name}:`,
                      hookError
                    );
                  });
                } catch (hookError) {
                  console.warn(
                    `⚠️  Background step-level onFailure hook failed for ${step.name}:`,
                    hookError
                  );
                }
              }
            }
          })
          .catch(error => {
            console.error(
              `❌ ${step.type === 'action' ? 'Action' : 'Step'} ${step.name} failed in background:`,
              error
            );

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
                ).catch(hookError => {
                  console.warn(
                    `⚠️  Background step-level onFailure hook failed for ${step.name}:`,
                    hookError
                  );
                });
              } catch (hookError) {
                console.warn(
                  `⚠️  Background step-level onFailure hook failed for ${step.name}:`,
                  hookError
                );
              }
            }
          });

        // For background steps/actions, we don't wait for completion and don't store results
        // The workflow continues immediately to the next step
        console.log(
          `⏭️  Continuing workflow execution while ${step.type === 'action' ? 'action' : 'step'} ${step.name} runs in background`
        );
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
        console.log(`✅ Step ${step.name} completed successfully`);

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
          } catch (hookError) {
            console.warn(
              `⚠️  Step-level onSuccess hook failed for step ${step.name}:`,
              hookError
            );
          }
        }
      } else {
        console.error(`❌ Step ${step.name} failed:`, stepResult.message);

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
          } catch (hookError) {
            console.warn(
              `⚠️  Step-level onFailure hook failed for step ${step.name}:`,
              hookError
            );
          }
        }

        throw new Error(`Step ${step.name} failed: ${stepResult.message}`);
      }
    }

    console.log(`✅ Workflow steps executed successfully for run ${runId}`);
  } catch (error) {
    console.error(`❌ Error executing workflow steps:`, error);
    throw error;
  }
}

export async function inspect(runId: string): Promise<any> {
  const currentState = getCurrentState();

  if (!core) {
    console.log(`⚠️  Simulation: Inspecting run: ${runId}`);
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
    console.error(`❌ Failed to inspect run ${runId}:`, error);
    throw error;
  }
}

export async function cancelRun(runId: string): Promise<void> {
  console.log(`Cancelling run: ${runId}`);
  // TODO: Implement run cancellation
}

export async function publishEvent(name: string, payload: any): Promise<void> {
  console.log(`📡 Publishing event: ${name}`, payload);

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
    console.log(`ℹ️  No workflows listening to event: ${name}`);
    return;
  }

  console.log(
    `🚀 Triggering ${listeners.length} workflow(s) for event: ${name}`
  );

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

      console.log(`   🔄 Triggering workflow: ${workflowId}`);
      const runId = await trigger(workflowId, eventPayload);
      console.log(
        `   ✅ Workflow ${workflowId} triggered successfully with run ID: ${runId}`
      );

      return { workflowId, runId, success: true };
    } catch (error) {
      console.error(
        `   ❌ Failed to trigger workflow ${listener.workflowId}:`,
        error
      );
      return { workflowId: listener.workflowId, error, success: false };
    }
  });

  const results = await Promise.allSettled(triggerPromises);

  const successful = results.filter(
    r => r.status === 'fulfilled' && r.value.success
  ).length;
  const failed = results.length - successful;

  console.log(`📊 Event ${name} processing complete:`);
  console.log(`   ✅ Successfully triggered: ${successful} workflow(s)`);
  if (failed > 0) {
    console.log(`   ❌ Failed to trigger: ${failed} workflow(s)`);
  }

  results.forEach((result, index) => {
    if (result.status === 'rejected') {
      console.error(`   ❌ Workflow trigger failed:`, result.reason);
    } else if (result.status === 'fulfilled' && !result.value.success) {
      console.error(
        `   ❌ Workflow ${result.value.workflowId} failed:`,
        result.value.error
      );
    }
  });
}

export async function executeStep(
  runId: string,
  stepId: string,
  contextJson?: string
): Promise<any> {
  if (!core) {
    console.log(`⚠️  Simulation: Executing step ${stepId} for run ${runId}`);
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
    if (contextJson && contextJson.trim() !== '') {
      const contextData = JSON.parse(contextJson);
      const workflowId = contextData.workflow_id || contextData.workflowId;

      if (workflowId) {
        console.log(
          `🔄 Executing step function ${stepId} for workflow ${workflowId}`
        );
        return await executeStepFunction(
          stepId,
          contextJson,
          workflowId,
          runId
        );
      }
    }

    const currentState = getCurrentState();
    const workflow = currentState.workflows.get(stepId.split(':')[0] || '');

    let servicesJson = '{}';
    if (workflow && workflow.services) {
      const servicesObject: Record<string, any> = {};
      for (const service of workflow.services) {
        servicesObject[service.id] = {
          id: service.id,
          name: service.name,
          version: service.version,
          config: service.config,
          auth: service.auth,
        };
      }
      servicesJson = JSON.stringify(servicesObject);
    }

    const result = core.executeStep(
      runId,
      stepId,
      currentState.dbPath,
      servicesJson
    );

    if (result.success && result.result) {
      console.log(`✅ Step ${stepId} executed successfully for run ${runId}`);
      return JSON.parse(result.result);
    } else {
      throw new Error(`Failed to execute step: ${result.message}`);
    }
  } catch (error) {
    console.error(
      `❌ Failed to execute step ${stepId} for run ${runId}:`,
      error
    );
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
    console.log(
      `⚠️  Simulation: Executing step function ${stepName} for workflow ${workflowId} run ${runId}`
    );
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

    let services: Record<string, any> = contextData.services || {};
    if (workflow && workflow.services) {
      for (const service of workflow.services) {
        if (services[service.id]) {
          services[service.id] = {
            ...services[service.id],
            actions: service.actions,
          };
        } else {
          services[service.id] = {
            id: service.id,
            name: service.name,
            version: service.version,
            config: service.config,
            auth: service.auth,
            actions: service.actions,
          };
        }
      }
    }

    const enhancedContext = createEnhancedContext(
      contextData,
      services,
      workflowId,
      runId
    );

    console.log(`   - Executing step function ${stepName}...`);

    const originalConsoleLog = console.log;
    const originalConsoleError = console.error;
    const originalConsoleWarn = console.warn;
    const originalConsoleInfo = console.info;

    const capturedLogs: string[] = [];

    console.log = (...args: any[]) => {
      const message = args
        .map(arg =>
          typeof arg === 'object' ? JSON.stringify(arg, null, 2) : String(arg)
        )
        .join(' ');
      capturedLogs.push(`[LOG] ${message}`);
      originalConsoleLog(...args);
    };

    console.error = (...args: any[]) => {
      const message = args
        .map(arg =>
          typeof arg === 'object' ? JSON.stringify(arg, null, 2) : String(arg)
        )
        .join(' ');
      capturedLogs.push(`[ERROR] ${message}`);
      originalConsoleError(...args);
    };

    console.warn = (...args: any[]) => {
      const message = args
        .map(arg =>
          typeof arg === 'object' ? JSON.stringify(arg, null, 2) : String(arg)
        )
        .join(' ');
      capturedLogs.push(`[WARN] ${message}`);
      originalConsoleWarn(...args);
    };

    console.info = (...args: any[]) => {
      const message = args
        .map(arg =>
          typeof arg === 'object' ? JSON.stringify(arg, null, 2) : String(arg)
        )
        .join(' ');
      capturedLogs.push(`[INFO] ${message}`);
      originalConsoleInfo(...args);
    };

    try {
      const result = await stepHandler.handler(enhancedContext);

      console.log = originalConsoleLog;
      console.error = originalConsoleError;
      console.warn = originalConsoleWarn;
      console.info = originalConsoleInfo;

      if (capturedLogs.length > 0) {
        capturedLogs.forEach(log => {
          const message = log.replace(/^\[LOG\] /, '');
          console.log(message);
        });
      }

      console.log(`   ✅ Step function ${stepName} executed successfully`);

      const endTime = process.hrtime.bigint();
      const duration = Number(endTime - startTime) / 1000000; // Convert to milliseconds

      console.log(`   📊 Step execution metrics:`);
      console.log(`      - Duration: ${duration.toFixed(2)}ms`);
      console.log(`      - Context size: ${contextSize} bytes`);
      console.log(`      - Services: ${Object.keys(services).length}`);
      console.log(
        `      - Steps completed: ${Object.keys(contextData.steps || {}).length}`
      );

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
          logs: capturedLogs,
        },
        message: 'Step function executed successfully',
      };
    } catch (error) {
      console.log = originalConsoleLog;
      console.error = originalConsoleError;
      console.warn = originalConsoleWarn;
      console.info = originalConsoleInfo;

      if (capturedLogs.length > 0) {
        capturedLogs.forEach(log => {
          const message = log.replace(/^\[LOG\] /, '');
          console.log(message);
        });
      }

      throw error;
    }
  } catch (error: any) {
    const endTime = process.hrtime.bigint();
    const duration = Number(endTime - startTime) / 1000000;

    console.error(`   ❌ Error executing step function ${stepName}:`, error);
    console.error(`   📊 Error metrics:`);
    console.error(`      - Duration: ${duration.toFixed(2)}ms`);
    console.error(`      - Context size: ${contextJson.length} bytes`);

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
  services: Record<string, any>,
  workflowId: string,
  runId: string
): Context {
  const complexityScore = calculateContextComplexity(contextData);

  const enhancedContext: Context = {
    ...contextData,
    services,
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

  const servicesCount = Object.keys(contextData.services || {}).length;
  if (servicesCount > 10) score += 2;
  else if (servicesCount > 5) score += 1;

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
    console.log(`⚠️  Simulation: Executing job function with job: ${jobJson}`);
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
      console.log(`✅ Job function executed successfully`);
      return JSON.parse(result.result);
    } else {
      throw new Error(`Failed to execute job function: ${result.message}`);
    }
  } catch (error) {
    console.error(`❌ Failed to execute job function:`, error);
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
    console.log(`⚠️  Simulation: Registering workflow: ${workflow.id}`);
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

    console.log(
      `✅ Workflow '${workflow.id}' registered successfully with Rust engine`
    );
  } catch (error) {
    console.error(`❌ Failed to register workflow '${workflow.id}':`, error);
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
      action: step.handler.toString(), // Always include action field
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
        return {
          Schedule: {
            cron_expression: trigger.cron_expression,
          },
        };
      } else if (trigger.type === 'event') {
        // Event triggers are handled in the Node.js layer, not Rust
        // For now, treat them as manual triggers for Rust compatibility
        return { Manual: {} };
      } else {
        return { Manual: {} };
      }
    }),
    // Note: Services are not serialized to Rust as they contain functions
    // Services are handled in the Node.js layer during execution
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
  console.log(`Replaying run: ${runId} with options:`, options);
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
  console.log(`🔄 Resuming workflow with token: ${token}`);
  console.log(`📋 Resume payload:`, payload);

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

  console.log(`✅ Workflow resumed successfully with token: ${token}`);
  console.log(`📊 Resume info:`, {
    token,
    payload,
    workflowId: pausedWorkflow.workflowId,
    runId: pausedWorkflow.runId,
    resumedAt: pausedWorkflow.resumedAt,
    status: 'resumed',
  });

  if (pausedWorkflow.resumeCallback) {
    pausedWorkflow.resumeCallback(payload);
  } else {
    console.log(`🔄 Continuing workflow execution from pause step`);

    try {
      await executeWorkflowSteps(
        pausedWorkflow.workflowId,
        pausedWorkflow.runId,
        pausedWorkflow.payload
      );

      console.log(`✅ Workflow execution completed after resume`);
    } catch (error) {
      console.error(`❌ Error continuing workflow execution:`, error);
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
    rss: Math.round(usage.rss / 1024 / 1024), // MB
    heapTotal: Math.round(usage.heapTotal / 1024 / 1024), // MB
    heapUsed: Math.round(usage.heapUsed / 1024 / 1024), // MB
    external: Math.round(usage.external / 1024 / 1024), // MB
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

  if (verbose) {
    console.log('🚀 Node-Cronflow Performance Benchmark');
    console.log('='.repeat(60));
    console.log(`📊 Running ${iterations} iterations...`);
    console.log(`⏱️  Start Time: ${new Date().toISOString()}`);
    console.log(
      `🔧 Configuration: ${stepsPerWorkflow} steps/workflow, ${payloadSize} items/payload`
    );
    console.log('─'.repeat(60));
  }

  const results: any[] = [];
  const durations: number[] = [];
  const memoryDeltas: number[] = [];
  const benchmarkId = Date.now().toString();

  for (let i = 1; i <= iterations; i++) {
    if (verbose) {
      console.log(`🔄 Running iteration ${i}/${iterations}...`);
    }

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
      const duration = Number(endTime - startTime) / 1000000; // Convert to ms

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

  if (verbose) {
    console.log('─'.repeat(60));
    console.log('📈 PERFORMANCE BENCHMARK RESULTS');
    console.log('─'.repeat(60));

    if (successfulRuns > 0) {
      console.log('⏱️  Execution Time Statistics:');
      console.log(`   - Iterations: ${durationStats.count}`);
      console.log(`   - Mean: ${formatDuration(durationStats.mean)}`);
      console.log(`   - Median: ${formatDuration(durationStats.median)}`);
      console.log(`   - Min: ${formatDuration(durationStats.min)}`);
      console.log(`   - Max: ${formatDuration(durationStats.max)}`);
      console.log(`   - Std Dev: ${formatDuration(durationStats.stdDev)}`);
      console.log(`   - 95th Percentile: ${formatDuration(durationStats.p95)}`);
      console.log(`   - 99th Percentile: ${formatDuration(durationStats.p99)}`);

      console.log('\n📊 Memory Usage Statistics:');
      console.log(`   - Mean Heap Delta: ${memoryStats.mean.toFixed(2)} MB`);
      console.log(
        `   - Median Heap Delta: ${memoryStats.median.toFixed(2)} MB`
      );
      console.log(`   - Min Heap Delta: ${memoryStats.min.toFixed(2)} MB`);
      console.log(`   - Max Heap Delta: ${memoryStats.max.toFixed(2)} MB`);

      console.log('\n🚀 Performance Metrics:');
      console.log(`   - Average Steps/Second: ${stepsPerSecond.toFixed(2)}`);
      console.log(`   - Average Step Time: ${formatDuration(averageStepTime)}`);
      console.log(`   - Throughput: ${throughput.toFixed(2)} workflows/second`);
    } else {
      console.log('❌ No successful runs to analyze');
    }

    console.log('\n📋 Success Rate:');
    console.log(
      `   - Successful: ${successfulRuns}/${iterations} (${successRate.toFixed(1)}%)`
    );

    if (successfulRuns < iterations) {
      console.log('\n❌ Failed Runs:');
      results
        .filter(r => !r.success)
        .forEach(r => {
          console.log(`   - Iteration ${r.iteration}: ${r.error}`);
        });
    }

    console.log('─'.repeat(60));
    console.log('✅ Benchmark completed');
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
    console.log(
      `⚠️  Simulation: Executing manual trigger for workflow: ${workflowId}`
    );
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
      console.log(
        `✅ Manual trigger executed successfully for workflow: ${workflowId}`
      );
      return result;
    } else {
      throw new Error(`Failed to execute manual trigger: ${result.message}`);
    }
  } catch (error) {
    console.error(
      `❌ Failed to execute manual trigger for workflow ${workflowId}:`,
      error
    );
    throw error;
  }
}

export async function executeWebhookTrigger(request: any): Promise<any> {
  if (!core) {
    console.log(
      `⚠️  Simulation: Executing webhook trigger for request:`,
      request
    );
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
      console.log(`✅ Webhook trigger executed successfully`);
      return result;
    } else {
      throw new Error(`Failed to execute webhook trigger: ${result.message}`);
    }
  } catch (error) {
    console.error(`❌ Failed to execute webhook trigger:`, error);
    throw error;
  }
}

export async function executeScheduleTrigger(triggerId: string): Promise<any> {
  if (!core) {
    console.log(`⚠️  Simulation: Executing schedule trigger: ${triggerId}`);
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
      console.log(`✅ Schedule trigger executed successfully: ${triggerId}`);
      return result;
    } else {
      throw new Error(`Failed to execute schedule trigger: ${result.message}`);
    }
  } catch (error) {
    console.error(`❌ Failed to execute schedule trigger ${triggerId}:`, error);
    throw error;
  }
}

export async function getTriggerStats(): Promise<any> {
  if (!core) {
    console.log(`⚠️  Simulation: Getting trigger statistics`);
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
      console.log(`✅ Trigger statistics retrieved successfully`);
      return result;
    } else {
      throw new Error(`Failed to get trigger stats: ${result.message}`);
    }
  } catch (error) {
    console.error(`❌ Failed to get trigger statistics:`, error);
    throw error;
  }
}

export async function getWorkflowTriggers(workflowId: string): Promise<any> {
  if (!core) {
    console.log(`⚠️  Simulation: Getting triggers for workflow: ${workflowId}`);
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
      console.log(`✅ Workflow triggers retrieved successfully`);
      return result;
    } else {
      throw new Error(`Failed to get workflow triggers: ${result.message}`);
    }
  } catch (error) {
    console.error(`❌ Failed to get workflow triggers:`, error);
    throw error;
  }
}

export async function unregisterWorkflowTriggers(
  workflowId: string
): Promise<any> {
  if (!core) {
    console.log(
      `⚠️  Simulation: Unregistering triggers for workflow: ${workflowId}`
    );
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
      console.log(`✅ Workflow triggers unregistered successfully`);
      return result;
    } else {
      throw new Error(
        `Failed to unregister workflow triggers: ${result.message}`
      );
    }
  } catch (error) {
    console.error(`❌ Failed to unregister workflow triggers:`, error);
    throw error;
  }
}

export async function getScheduleTriggers(): Promise<any> {
  if (!core) {
    console.log(`⚠️  Simulation: Getting schedule triggers`);
    return {
      success: true,
      triggers: JSON.stringify([]),
      message: 'Schedule triggers retrieved in simulation mode',
    };
  }

  try {
    const result = core.getScheduleTriggers(getCurrentState().dbPath);

    if (result.success) {
      console.log(`✅ Schedule triggers retrieved successfully`);
      return result;
    } else {
      throw new Error(`Failed to get schedule triggers: ${result.message}`);
    }
  } catch (error) {
    console.error(`❌ Failed to get schedule triggers:`, error);
    throw error;
  }
}

export function createValidContext(
  runId: string,
  workflowId: string,
  stepName: string,
  payload: any = {},
  steps: Record<string, any> = {},
  services: Record<string, any> = {},
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
    steps: {}, // Empty object for now - StepResult requires complex structure
    services: services,
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
  console.log(`🎣 Registered ${hookType} hook handler`);
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
    console.log(
      `🎣 Executing ${hookType} hook for workflow: ${workflowId}${stepId ? `, step: ${stepId}` : ''}`
    );

    if (hookType !== 'onSuccess' && hookType !== 'onFailure') {
      console.warn(`⚠️  Invalid hook type: ${hookType}`);
      return { success: false, error: `Invalid hook type: ${hookType}` };
    }

    const context = JSON.parse(contextJson);
    console.log(`   - Run ID: ${context.run_id}`);
    console.log(`   - Status: ${context.status}`);
    console.log(`   - Duration: ${context.duration_ms}ms`);
    console.log(
      `   - Completed steps: ${context.completed_steps?.length || 0}`
    );

    const workflow = getWorkflow(workflowId);
    if (!workflow) {
      console.warn(`⚠️  Workflow ${workflowId} not found for hook execution`);
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
        if (stepId) {
          // For step-level hooks, we need to check if the hook should run for this specific step
          // The hook function signature is (ctx: Context, stepId?: string | string[])
          // We need to determine if the hook should execute for this stepId

          // If stepId is provided, we need to check if the hook should run for this step
          // The hook might be configured to run for specific steps or all steps
          console.log(
            `   - Checking if ${hookType} hook should run for step: ${stepId}`
          );

          // For now, we'll execute the hook and let it decide internally
          // The hook function can check the stepId parameter to determine if it should run
        }

        console.log(`   - Executing ${hookType} hook...`);
        await hook(context, stepId);
        console.log(`   ✅ ${hookType} hook executed successfully`);
        return {
          success: true,
          message: `${hookType} hook executed successfully`,
          hookType,
          workflowId,
          stepId,
        };
      } catch (error: any) {
        console.error(`   ❌ Error executing ${hookType} hook:`, error);
        return {
          success: false,
          error: error.message,
          hookType,
          workflowId,
          stepId,
        };
      }
    } else {
      console.log(`   - No ${hookType} hook defined for workflow`);
      return {
        success: true,
        message: `No ${hookType} hook defined`,
        hookType,
        workflowId,
        stepId,
      };
    }
  } catch (error: any) {
    console.error(`❌ Failed to execute ${hookType} hook:`, error);
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
    console.log(
      `🔄 Updated event listener for workflow ${workflowId} on event: ${eventName}`
    );
  } else {
    listeners.push({ workflowId, trigger });
    console.log(
      `✅ Registered workflow ${workflowId} to listen for event: ${eventName}`
    );
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
    console.log(
      `🗑️  Unregistered workflow ${workflowId} from event: ${eventName}`
    );

    if (listeners.length === 0) {
      currentState.eventListeners.delete(eventName);
      console.log(`🧹 Cleaned up empty event: ${eventName}`);
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
  VERSION,
  // Event handling functions
  registerEventListener,
  unregisterEventListener,
  getEventHistory,
  getEventListeners,
};
