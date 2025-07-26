import { describe, it, expect, beforeEach } from 'vitest';
import { cronflow, WorkflowInstance } from '../sdk/index';
import { Context } from '../sdk/src/workflow/types';

describe('Workflow Builder', () => {
  let workflow: WorkflowInstance;
  let testCounter = 0;

  beforeEach(async () => {
    // Reset cronflow instance for each test
    try {
      await cronflow.stop();
    } catch (error) {
      // Ignore errors if already stopped
    }

    // Clear any existing workflows by accessing the internal state
    const currentState = (cronflow as any).getCurrentState?.() || {};
    if (currentState.workflows) {
      currentState.workflows.clear();
    }

    // Create a new workflow with a unique ID for each test
    testCounter++;
    workflow = cronflow.define({
      id: `test-workflow-${testCounter}`,
      name: 'Test Workflow',
    });
  });

  describe('cronflow.define()', () => {
    it('should create a workflow with basic definition', () => {
      expect(workflow).toBeDefined();
      expect(workflow.getId()).toBe(`test-workflow-${testCounter}`);
      expect(workflow.getName()).toBe('Test Workflow');
      expect(workflow.getSteps()).toEqual([]);
      expect(workflow.getTriggers()).toEqual([]);
    });

    it('should throw error for empty ID', () => {
      expect(() => {
        cronflow.define({
          id: '',
          name: 'Test Workflow',
        });
      }).toThrow('Workflow ID cannot be empty');
    });

    it('should throw error for duplicate workflow ID', () => {
      cronflow.define({
        id: 'duplicate-test-workflow',
        name: 'Test Workflow',
      });

      expect(() => {
        cronflow.define({
          id: 'duplicate-test-workflow',
          name: 'Another Workflow',
        });
      }).toThrow("Workflow with ID 'duplicate-test-workflow' already exists");
    });

    it('should accept setup function', () => {
      const setupWorkflow = cronflow.define({
        id: 'setup-test-workflow',
        name: 'Test Workflow',
      });

      // Use the fluent API after creation
      setupWorkflow
        .step('test-step', async ctx => {
          return { result: 'test' };
        })
        .onWebhook('/webhook/test');

      expect(setupWorkflow).toBeInstanceOf(WorkflowInstance);
      expect(setupWorkflow.getId()).toBe('setup-test-workflow');
    });
  });

  describe('WorkflowInstance fluent API', () => {
    let workflow: WorkflowInstance;
    let fluentApiCounter = 0;

    beforeEach(() => {
      fluentApiCounter++;
      workflow = cronflow.define({
        id: `fluent-api-workflow-${fluentApiCounter}`,
        name: 'Test Workflow',
      });
    });

    it('should add steps with fluent API', () => {
      workflow
        .step('step1', async ctx => {
          return { result: 'step1' };
        })
        .step('step2', async ctx => {
          return { result: 'step2' };
        });

      const steps = workflow.getSteps();
      expect(steps).toHaveLength(2);
      expect(steps[0].id).toBe('step1');
      expect(steps[0].name).toBe('step1');
      expect(steps[0].type).toBe('step');
      expect(steps[1].id).toBe('step2');
      expect(steps[1].name).toBe('step2');
      expect(steps[1].type).toBe('step');
    });

    it('should add actions with fluent API', () => {
      workflow
        .action('action1', async ctx => {
          console.log('Action 1');
        })
        .action('action2', async ctx => {
          console.log('Action 2');
        });

      const steps = workflow.getSteps();
      expect(steps).toHaveLength(2);
      expect(steps[0].type).toBe('action');
      expect(steps[1].type).toBe('action');
    });

    it('should set timeout for current step', () => {
      workflow.step(
        'step1',
        async ctx => {
          return { result: 'step1' };
        },
        {
          timeout: '30s',
        }
      );

      const step = workflow.getStep('step1');
      expect(step?.options?.timeout).toBe('30s');
    });

    it('should set retry configuration for current step', () => {
      workflow.step(
        'step1',
        async ctx => {
          return { result: 'step1' };
        },
        {
          retry: {
            attempts: 3,
            backoff: { strategy: 'exponential', delay: '1s' },
          },
        }
      );

      const step = workflow.getStep('step1');
      expect(step?.options?.retry).toEqual({
        attempts: 3,
        backoff: { strategy: 'exponential', delay: '1s' },
      });
    });

    it('should add webhook trigger', () => {
      workflow.onWebhook('/webhook/test', {
        method: 'POST',
        parseRawBody: true,
      });

      const triggers = workflow.getTriggers();
      expect(triggers).toHaveLength(1);
      expect(triggers[0]).toEqual({
        type: 'webhook',
        path: '/webhook/test',
        options: {
          method: 'POST',
          parseRawBody: true,
        },
      });
    });

    it('should add schedule trigger', () => {
      workflow.onSchedule('0 0 * * *');

      const triggers = workflow.getTriggers();
      expect(triggers).toHaveLength(1);
      expect(triggers[0]).toEqual({
        type: 'schedule',
        cron_expression: '0 0 * * *',
      });
    });

    it('should add interval trigger', () => {
      workflow.onInterval('5m');

      const triggers = workflow.getTriggers();
      expect(triggers).toHaveLength(1);
      expect(triggers[0]).toEqual({
        type: 'schedule',
        cron_expression: '*/5 * * * *',
      });
    });

    it('should add manual trigger', () => {
      workflow.manual();

      const triggers = workflow.getTriggers();
      expect(triggers).toHaveLength(1);
      expect(triggers[0]).toEqual({
        type: 'manual',
      });
    });

    it('should validate workflow successfully', () => {
      workflow
        .step('step1', async ctx => {
          return { result: 'step1' };
        })
        .onWebhook('/webhook/test');

      expect(() => {
        workflow.validate();
      }).not.toThrow();
    });

    it('should throw error for invalid workflow (no steps)', () => {
      workflow.onWebhook('/webhook/test');

      expect(() => {
        workflow.validate();
      }).toThrow('Workflow must have at least one step');
    });

    it('should serialize workflow to JSON', () => {
      workflow
        .step(
          'step1',
          async ctx => {
            return { result: 'step1' };
          },
          {
            timeout: '30s',
            retry: {
              attempts: 3,
              backoff: { strategy: 'exponential', delay: '1s' },
            },
          }
        )
        .onWebhook('/webhook/test', {
          method: 'POST',
        });

      const json = workflow.toJSON();
      const parsed = JSON.parse(json);

      expect(parsed.id).toBe(`fluent-api-workflow-${fluentApiCounter}`);
      expect(parsed.name).toBe('Test Workflow');
      expect(parsed.steps).toHaveLength(1);
      expect(parsed.triggers).toHaveLength(1);
    });

    it('should check trigger types correctly', () => {
      expect(workflow.hasWebhookTriggers()).toBe(false);
      expect(workflow.hasScheduleTriggers()).toBe(false);
      expect(workflow.hasManualTriggers()).toBe(false);

      workflow.onWebhook('/webhook/test');
      expect(workflow.hasWebhookTriggers()).toBe(true);

      workflow.onSchedule('0 0 * * *');
      expect(workflow.hasScheduleTriggers()).toBe(true);

      workflow.manual();
      expect(workflow.hasManualTriggers()).toBe(true);
    });

    it('should get step by ID', () => {
      workflow
        .step('step1', async ctx => {
          return { result: 'step1' };
        })
        .step('step2', async ctx => {
          return { result: 'step2' };
        });

      const step1 = workflow.getStep('step1');
      const step2 = workflow.getStep('step2');
      const step3 = workflow.getStep('step3');

      expect(step1?.id).toBe('step1');
      expect(step2?.id).toBe('step2');
      expect(step3).toBeUndefined();
    });

    it('should add logging step', () => {
      workflow.log('Test log message');

      const steps = workflow.getSteps();
      expect(steps).toHaveLength(1);
      expect(steps[0].type).toBe('action');
      expect(steps[0].name).toBe('log');
    });

    it('should add sleep step', () => {
      workflow.sleep('5s');

      const steps = workflow.getSteps();
      expect(steps).toHaveLength(1);
      expect(steps[0].type).toBe('action');
      expect(steps[0].name).toBe('sleep');
    });
  });

  describe('Control Flow Methods', () => {
    it('should implement if/elseIf/else/endIf conditional flow', () => {
      const workflow = cronflow.define({ id: 'conditional-workflow' });

      workflow
        .if('is-high-value', ctx => ctx.payload.amount > 500)
        .step('send-vip-notification', async ctx => {
          return { type: 'vip', message: 'High value order' };
        })
        .elseIf('is-medium-value', ctx => ctx.payload.amount > 100)
        .step('send-standard-notification', async ctx => {
          return { type: 'standard', message: 'Medium value order' };
        })
        .else()
        .step('send-basic-notification', async ctx => {
          return { type: 'basic', message: 'Basic order' };
        })
        .endIf();

      expect(workflow.getSteps()).toHaveLength(7); // 3 conditional steps + 3 regular steps + 1 endIf

      const steps = workflow.getSteps();
      expect(steps[0].name).toBe('if_is-high-value');
      expect(steps[0].options?.conditional).toBe(true);
      expect(steps[0].options?.conditionType).toBe('if');

      expect(steps[1].name).toBe('send-vip-notification');

      expect(steps[2].name).toBe('elseif_is-medium-value');
      expect(steps[2].options?.conditional).toBe(true);
      expect(steps[2].options?.conditionType).toBe('elseIf');

      expect(steps[3].name).toBe('send-standard-notification');

      expect(steps[4].name).toBe('else_is-medium-value');
      expect(steps[4].options?.conditional).toBe(true);
      expect(steps[4].options?.conditionType).toBe('else');

      expect(steps[5].name).toBe('send-basic-notification');

      expect(steps[6].name).toBe('endif_is-medium-value');
      expect(steps[6].options?.controlFlow).toBe(true);
      expect(steps[6].options?.endIf).toBe(true);
    });

    it('should implement parallel execution', () => {
      const workflow = cronflow.define({ id: 'parallel-workflow' });

      workflow.parallel([
        ctx => Promise.resolve({ type: 'data1', value: 1 }),
        ctx => Promise.resolve({ type: 'data2', value: 2 }),
        ctx => Promise.resolve({ type: 'data3', value: 3 }),
      ]);

      const steps = workflow.getSteps();
      expect(steps).toHaveLength(1);

      const parallelStep = steps[0];
      expect(parallelStep.name).toBe('parallel_execution');
      expect(parallelStep.options?.parallel).toBe(true);
      expect(parallelStep.options?.stepCount).toBe(3);
    });

    it('should implement race execution', () => {
      const workflow = cronflow.define({ id: 'race-workflow' });

      workflow.race([
        ctx =>
          new Promise(resolve =>
            setTimeout(() => resolve({ winner: 'first' }), 100)
          ),
        ctx =>
          new Promise(resolve =>
            setTimeout(() => resolve({ winner: 'second' }), 50)
          ),
        ctx =>
          new Promise(resolve =>
            setTimeout(() => resolve({ winner: 'third' }), 200)
          ),
      ]);

      const steps = workflow.getSteps();
      expect(steps).toHaveLength(1);

      const raceStep = steps[0];
      expect(raceStep.name).toBe('race_execution');
      expect(raceStep.options?.race).toBe(true);
      expect(raceStep.options?.stepCount).toBe(3);
    });

    it('should implement while loop', () => {
      const workflow = cronflow.define({ id: 'while-workflow' });

      workflow.while(
        'process-queue',
        ctx => ctx.state.get('queue-size', 0) > 0,
        ctx => {
          const currentSize = ctx.state.get('queue-size', 0);
          ctx.state.set('queue-size', currentSize - 1);
        }
      );

      const steps = workflow.getSteps();
      expect(steps).toHaveLength(1);

      const whileStep = steps[0];
      expect(whileStep.name).toBe('while_process-queue');
      expect(whileStep.options?.loop).toBe(true);
      expect(whileStep.options?.maxIterations).toBe(1000);
    });

    it('should throw error for unmatched endIf', () => {
      const workflow = cronflow.define({ id: 'invalid-endif-workflow' });

      expect(() => {
        workflow.endIf();
      }).toThrow('endIf() called without matching if()');
    });

    it('should throw error for unmatched elseIf', () => {
      const workflow = cronflow.define({ id: 'invalid-elseif-workflow' });

      expect(() => {
        workflow.elseIf('test', ctx => true);
      }).toThrow('elseIf() called without matching if()');
    });

    it('should throw error for unmatched else', () => {
      const workflow = cronflow.define({ id: 'invalid-else-workflow' });

      expect(() => {
        workflow.else();
      }).toThrow('else() called without matching if()');
    });

    it('should handle complex nested control flow', () => {
      const workflow = cronflow.define({ id: 'nested-control-flow' });

      workflow
        .if('outer-condition', ctx => ctx.payload.type === 'order')
        .step('process-order', async ctx => ({ processed: true }))
        .if('inner-condition', ctx => ctx.last.processed)
        .step('send-confirmation', async ctx => ({ confirmed: true }))
        .endIf()
        .else()
        .step('process-other', async ctx => ({ other: true }))
        .endIf();

      const steps = workflow.getSteps();
      expect(steps).toHaveLength(8); // 2 if steps + 3 regular steps + 2 endIf steps + 1 else step

      // Verify the structure
      expect(steps[0].name).toBe('if_outer-condition');
      expect(steps[1].name).toBe('process-order');
      expect(steps[2].name).toBe('if_inner-condition');
      expect(steps[3].name).toBe('send-confirmation');
      expect(steps[4].name).toBe('endif_inner-condition');
      expect(steps[5].name).toBe('else_outer-condition');
      expect(steps[6].name).toBe('process-other');
      expect(steps[7].name).toBe('endif_outer-condition');
    });
  });

  describe('Complex workflow example', () => {
    it('should create a complex workflow with all features', () => {
      const workflow = cronflow.define({
        id: 'email-workflow',
        name: 'Email Processing Workflow',
        description: 'Process incoming emails and send notifications',
        tags: ['email', 'processing'],
        concurrency: 10,
        timeout: '5m',
        hooks: {
          onSuccess: ctx => {
            console.log('Workflow succeeded');
          },
          onFailure: ctx => {
            console.error('Workflow failed');
          },
        },
      });

      // Use the fluent API after creation
      workflow
        .onWebhook('/webhook/email', {
          method: 'POST',
          parseRawBody: true,
        })
        .onSchedule('0 */5 * * *')
        .step(
          'fetch-emails',
          async ctx => {
            return { emails: [] };
          },
          {
            timeout: '2m',
            retry: {
              attempts: 3,
              backoff: { strategy: 'exponential', delay: '5s' },
            },
          }
        )
        .step(
          'process-emails',
          async ctx => {
            const { emails } = ctx.steps['fetch-emails'].output;
            return { processed: emails.length };
          },
          {
            timeout: '3m',
          }
        )
        .step(
          'send-notifications',
          async ctx => {
            const { processed } = ctx.steps['process-emails'].output;
            return { sent: processed };
          },
          {
            timeout: '1m',
            retry: {
              attempts: 2,
              backoff: { strategy: 'fixed', delay: '3s' },
            },
          }
        );

      // Verify workflow structure
      expect(workflow.getId()).toBe('email-workflow');
      expect(workflow.getName()).toBe('Email Processing Workflow');
      expect(workflow.getDefinition().description).toBe(
        'Process incoming emails and send notifications'
      );

      // Verify steps
      const steps = workflow.getSteps();
      expect(steps).toHaveLength(3);

      const fetchStep = workflow.getStep('fetch-emails');
      expect(fetchStep?.options?.timeout).toBe('2m');
      expect(fetchStep?.options?.retry).toEqual({
        attempts: 3,
        backoff: { strategy: 'exponential', delay: '5s' },
      });

      const processStep = workflow.getStep('process-emails');
      expect(processStep?.options?.timeout).toBe('3m');

      const notifyStep = workflow.getStep('send-notifications');
      expect(notifyStep?.options?.timeout).toBe('1m');
      expect(notifyStep?.options?.retry).toEqual({
        attempts: 2,
        backoff: { strategy: 'fixed', delay: '3s' },
      });

      // Verify triggers
      const triggers = workflow.getTriggers();
      expect(triggers).toHaveLength(2);
      expect(workflow.hasWebhookTriggers()).toBe(true);
      expect(workflow.hasScheduleTriggers()).toBe(true);

      // Verify validation
      expect(() => workflow.validate()).not.toThrow();

      // Verify JSON serialization
      const json = workflow.toJSON();
      const parsed = JSON.parse(json);
      expect(parsed.id).toBe('email-workflow');
      expect(parsed.steps).toHaveLength(3);
      expect(parsed.triggers).toHaveLength(2);
    });
  });
});
