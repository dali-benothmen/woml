import { TestHarness, TestAssertionBuilder } from './harness';
import { WorkflowDefinition, Context } from '../workflow';

export interface TriggerMock {
  type: 'webhook' | 'schedule' | 'manual';
  payload?: any;
  headers?: Record<string, string>;
  query?: Record<string, string>;
  path?: string;
}

export interface TestDataGenerator {
  generateUser(): { id: string; name: string; email: string; role: string };
  generateOrder(): {
    id: string;
    amount: number;
    items: any[];
    customerId: string;
  };
  generatePayment(): {
    id: string;
    amount: number;
    currency: string;
    status: string;
  };
  generateProduct(): {
    id: string;
    name: string;
    price: number;
    category: string;
  };
  generateEvent(): { id: string; type: string; data: any; timestamp: number };
  generateArray<T>(generator: () => T, count: number): T[];
  generateObject<T>(generator: () => T, keys: string[]): Record<string, T>;
}

export interface TestCoverage {
  steps: Record<
    string,
    {
      executed: boolean;
      executionCount: number;
      lastExecuted?: Date;
      duration?: number;
    }
  >;
  branches: Record<
    string,
    {
      executed: boolean;
      executionCount: number;
      condition: string;
    }
  >;
  overall: {
    totalSteps: number;
    executedSteps: number;
    totalBranches: number;
    executedBranches: number;
    coveragePercentage: number;
  };
}

export class AdvancedTestHarness extends TestHarness {
  private triggerMocks: TriggerMock[] = [];
  private dataGenerator: TestDataGenerator;
  private coverage!: TestCoverage;

  constructor(workflow: WorkflowDefinition) {
    super(workflow);
    this.dataGenerator = new DefaultTestDataGenerator();
    this.initializeCoverage();
  }

  trigger(payload: any): AdvancedTestHarness {
    super.trigger(payload);
    return this;
  }

  mockStep(
    stepName: string,
    mockFn: (ctx: Context) => any | Promise<any>
  ): AdvancedTestHarness {
    super.mockStep(stepName, mockFn);
    return this;
  }

  expectStep(stepName: string): TestAssertionBuilder {
    return super.expectStep(stepName);
  }

  mockTrigger(
    type: 'webhook' | 'schedule' | 'manual',
    options?: {
      payload?: any;
      headers?: Record<string, string>;
      query?: Record<string, string>;
      path?: string;
    }
  ): AdvancedTestHarness {
    const mock: TriggerMock = {
      type,
      payload: options?.payload,
      headers: options?.headers,
      query: options?.query,
      path: options?.path,
    };

    this.triggerMocks.push(mock);
    return this;
  }

  withDataGenerator(generator: TestDataGenerator): AdvancedTestHarness {
    this.dataGenerator = generator;
    return this;
  }

  generateData<T>(generatorFn: (gen: TestDataGenerator) => T): T {
    return generatorFn(this.dataGenerator);
  }

  getCoverage(): TestCoverage {
    return this.coverage;
  }

  resetCoverage(): AdvancedTestHarness {
    this.initializeCoverage();
    return this;
  }

  async run(): Promise<any> {
    const testRun = await super.run();
    this.updateCoverage(testRun);

    return {
      ...testRun,
      coverage: this.coverage,
      triggerMocks: this.triggerMocks,
    };
  }

  private initializeCoverage(): void {
    this.coverage = {
      steps: {},
      branches: {},
      overall: {
        totalSteps: 0,
        executedSteps: 0,
        totalBranches: 0,
        executedBranches: 0,
        coveragePercentage: 0,
      },
    };

    for (const step of this.workflow.steps) {
      this.coverage.steps[step.name] = {
        executed: false,
        executionCount: 0,
      };
    }
  }

  private updateCoverage(testRun: any): void {
    for (const [stepName, step] of Object.entries(testRun.steps)) {
      if (this.coverage.steps[stepName]) {
        const stepData = step as any;
        this.coverage.steps[stepName].executed =
          stepData.status === 'completed';
        this.coverage.steps[stepName].executionCount++;
        this.coverage.steps[stepName].lastExecuted = new Date();
        this.coverage.steps[stepName].duration = stepData.duration;
      }
    }

    this.coverage.overall.executedSteps = Object.values(
      this.coverage.steps
    ).filter(step => step.executed).length;

    this.coverage.overall.coveragePercentage =
      (this.coverage.overall.executedSteps / this.coverage.overall.totalSteps) *
      100;
  }
}

export class DefaultTestDataGenerator implements TestDataGenerator {
  generateUser(): { id: string; name: string; email: string; role: string } {
    const id = `user_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;
    const names = [
      'John Doe',
      'Jane Smith',
      'Bob Johnson',
      'Alice Brown',
      'Charlie Wilson',
    ];
    const name = names[Math.floor(Math.random() * names.length)];
    const email = `${name.toLowerCase().replace(' ', '.')}@example.com`;
    const roles = ['user', 'admin', 'moderator', 'guest'];
    const role = roles[Math.floor(Math.random() * roles.length)];

    return { id, name, email, role };
  }

  generateOrder(): {
    id: string;
    amount: number;
    items: any[];
    customerId: string;
  } {
    const id = `order_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;
    const amount = Math.floor(Math.random() * 1000) + 10;
    const itemCount = Math.floor(Math.random() * 5) + 1;
    const items = [];

    for (let i = 0; i < itemCount; i++) {
      items.push({
        id: `item_${i}`,
        name: `Product ${i + 1}`,
        price: Math.floor(Math.random() * 100) + 1,
        quantity: Math.floor(Math.random() * 3) + 1,
      });
    }

    const customerId = `customer_${Math.random().toString(36).substr(2, 9)}`;

    return { id, amount, items, customerId };
  }

  generatePayment(): {
    id: string;
    amount: number;
    currency: string;
    status: string;
  } {
    const id = `payment_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;
    const amount = Math.floor(Math.random() * 1000) + 10;
    const currencies = ['USD', 'EUR', 'GBP', 'JPY'];
    const currency = currencies[Math.floor(Math.random() * currencies.length)];
    const statuses = ['pending', 'completed', 'failed', 'cancelled'];
    const status = statuses[Math.floor(Math.random() * statuses.length)];

    return { id, amount, currency, status };
  }

  generateProduct(): {
    id: string;
    name: string;
    price: number;
    category: string;
  } {
    const id = `product_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;
    const names = [
      'Laptop',
      'Phone',
      'Tablet',
      'Headphones',
      'Keyboard',
      'Mouse',
      'Monitor',
    ];
    const name = names[Math.floor(Math.random() * names.length)];
    const price = Math.floor(Math.random() * 1000) + 50;
    const categories = ['Electronics', 'Computers', 'Accessories', 'Gaming'];
    const category = categories[Math.floor(Math.random() * categories.length)];

    return { id, name, price, category };
  }

  generateEvent(): { id: string; type: string; data: any; timestamp: number } {
    const id = `event_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;
    const types = [
      'user.created',
      'order.placed',
      'payment.processed',
      'product.updated',
    ];
    const type = types[Math.floor(Math.random() * types.length)];
    const data = { source: 'test', environment: 'testing' };
    const timestamp = Date.now();

    return { id, type, data, timestamp };
  }

  generateArray<T>(generator: () => T, count: number): T[] {
    return Array.from({ length: count }, () => generator());
  }

  generateObject<T>(generator: () => T, keys: string[]): Record<string, T> {
    const result: Record<string, T> = {};
    for (const key of keys) {
      result[key] = generator();
    }
    return result;
  }
}

export function createAdvancedTestHarness(
  workflow: WorkflowDefinition
): AdvancedTestHarness {
  return new AdvancedTestHarness(workflow);
}
