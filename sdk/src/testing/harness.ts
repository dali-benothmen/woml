import { WorkflowDefinition, Context, StepDefinition } from '../workflow/types';
import { StepExecutor, StepExecutionResult } from '../execution';

export interface TestAssertion {
  stepName: string;
  expectedStatus: 'succeed' | 'fail';
  expectedError?: string;
}

export interface TestStep {
  name: string;
  handler: (ctx: Context) => any | Promise<any>;
  originalHandler?: (ctx: Context) => any | Promise<any>;
  mocked: boolean;
}

export interface TestRun {
  status: 'completed' | 'failed';
  duration: number;
  steps: Array<{
    name: string;
    status: 'completed' | 'failed';
    output?: any;
    error?: Error;
    duration: number;
    attempts: number;
    retryDelays: number[];
  }>;
  error?: Error;
}

export class TestHarness {
  protected workflow: WorkflowDefinition;
  protected testSteps: Map<string, TestStep> = new Map();
  protected assertions: TestAssertion[] = [];
  protected payload: any = {};

  constructor(workflow: WorkflowDefinition) {
    this.workflow = workflow;
    this.initializeTestSteps();
  }

  private initializeTestSteps(): void {
    for (const step of this.workflow.steps) {
      this.testSteps.set(step.name, {
        name: step.name,
        handler: step.handler,
        originalHandler: step.handler,
        mocked: false,
      });
    }
  }

  trigger(payload: any): TestHarness {
    this.payload = payload;
    return this;
  }

  mockStep(
    stepName: string,
    mockHandler: (ctx: Context) => any | Promise<any>
  ): TestHarness {
    const testStep = this.testSteps.get(stepName);
    if (!testStep) {
      throw new Error(`Step '${stepName}' not found in workflow`);
    }

    testStep.handler = mockHandler;
    testStep.mocked = true;
    return this;
  }

  expectStep(stepName: string): TestAssertionBuilder {
    return new TestAssertionBuilder(this, stepName);
  }

  async run(): Promise<TestRun> {
    const startTime = Date.now();
    const context = this.createContext();
    const testRun: TestRun = {
      status: 'completed',
      duration: 0,
      steps: [],
    };

    try {
      for (const step of this.workflow.steps) {
        const testStep = this.testSteps.get(step.name);
        if (!testStep) {
          throw new Error(`Test step '${step.name}' not found`);
        }

        const stepStartTime = Date.now();
        const stepResult: TestRun['steps'][0] = {
          name: step.name,
          status: 'completed',
          duration: 0,
          attempts: 1,
          retryDelays: [],
        };

        try {
          // Execute step with retry logic
          const executionResult =
            await StepExecutor.executeStepWithErrorHandling(
              step,
              context,
              step.options
            );

          stepResult.status = executionResult.success ? 'completed' : 'failed';
          stepResult.output = executionResult.output;
          stepResult.error = executionResult.error;
          stepResult.duration = executionResult.totalDuration;
          stepResult.attempts = executionResult.attempts;
          stepResult.retryDelays = executionResult.retryDelays;

          if (executionResult.success) {
            context.last = executionResult.output;
            context.steps[step.name] = { output: executionResult.output };
          } else {
            throw executionResult.error;
          }
        } catch (error) {
          stepResult.status = 'failed';
          stepResult.error = error as Error;
          stepResult.duration = Date.now() - stepStartTime;

          const assertion = this.assertions.find(a => a.stepName === step.name);
          if (assertion && assertion.expectedStatus === 'fail') {
            if (assertion.expectedError) {
              const errorMessage = (error as Error).message;
              if (!errorMessage.includes(assertion.expectedError)) {
                throw new Error(
                  `Step '${step.name}' failed with unexpected error. Expected: ${assertion.expectedError}, Got: ${errorMessage}`
                );
              }
            }
          } else if (assertion && assertion.expectedStatus === 'succeed') {
            throw new Error(
              `Step '${step.name}' was expected to succeed but failed: ${(error as Error).message}`
            );
          } else {
            testRun.status = 'failed';
            testRun.error = error as Error;
            break;
          }
        }

        testRun.steps.push(stepResult);
      }
    } catch (error) {
      testRun.status = 'failed';
      testRun.error = error as Error;
    }

    testRun.duration = Date.now() - startTime;
    return testRun;
  }

  private createContext(): Context {
    return {
      payload: this.payload,
      steps: {},
      run: {
        id: 'test-run',
        workflowId: this.workflow.id,
      },
      state: {
        get: (key: string, defaultValue?: any) => defaultValue,
        set: async () => {},
        incr: async (key: string, amount: number = 1) => amount,
      },
      last: null,
      trigger: {
        headers: {},
      },
      cancel: (reason?: string) => {
        throw new Error(
          `Workflow cancelled: ${reason || 'No reason provided'}`
        );
      },
    };
  }

  public addAssertion(assertion: TestAssertion): void {
    this.assertions.push(assertion);
  }
}

export function createTestHarness(workflow: WorkflowDefinition): TestHarness {
  return new TestHarness(workflow);
}

export class TestAssertionBuilder {
  private harness: TestHarness;
  private stepName: string;

  constructor(harness: TestHarness, stepName: string) {
    this.harness = harness;
    this.stepName = stepName;
  }

  toSucceed(): TestHarness {
    this.harness.addAssertion({
      stepName: this.stepName,
      expectedStatus: 'succeed',
    });
    return this.harness;
  }

  toFail(): TestHarness {
    this.harness.addAssertion({
      stepName: this.stepName,
      expectedStatus: 'fail',
    });
    return this.harness;
  }

  toFailWith(errorMessage: string): TestHarness {
    this.harness.addAssertion({
      stepName: this.stepName,
      expectedStatus: 'fail',
      expectedError: errorMessage,
    });
    return this.harness;
  }
}
