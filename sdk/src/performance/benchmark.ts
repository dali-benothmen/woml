import { Context } from '../workflow/types';

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

let defineFunction: (options: any) => any = () => ({});
let startFunction: () => Promise<void> = async () => {};
let stopFunction: () => Promise<void> = async () => {};
let triggerFunction: (
  workflowId: string,
  payload: any
) => Promise<string> = async () => '';

export function setBenchmarkDependencies(
  define: (options: any) => any,
  start: () => Promise<void>,
  stop: () => Promise<void>,
  trigger: (workflowId: string, payload: any) => Promise<string>
): void {
  defineFunction = define;
  startFunction = start;
  stopFunction = stop;
  triggerFunction = trigger;
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
      const workflow = defineFunction({
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

      await startFunction();

      const payload = {
        iteration: i,
        timestamp: Date.now(),
        data: Array.from({ length: payloadSize }, (_, j) => `item-${j}`),
      };

      const runId = await triggerFunction(
        `benchmark-${benchmarkId}-${i}`,
        payload
      );

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
      await stopFunction();
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
