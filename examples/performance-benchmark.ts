#!/usr/bin/env bun

/**
 * Performance Benchmark for Node-Cronflow
 *
 * This script runs multiple iterations of the simple workflow test
 * to measure performance characteristics and provide detailed statistics.
 */

import { cronflow } from '../sdk/src/index';
import { Context } from '../sdk/src/workflow/types';

// Test configuration
const TEST_WORKFLOW_ID = 'performance-benchmark';
const ITERATIONS = 10;

// Performance monitoring utilities
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

async function runSingleBenchmark(iteration: number) {
  const startTime = process.hrtime.bigint();
  const startMemory = getMemoryUsage();

  try {
    // Create workflow
    const workflow = cronflow.define({
      id: `${TEST_WORKFLOW_ID}-${iteration}`,
      name: `Performance Benchmark ${iteration}`,
      description: 'Performance test workflow',
    });

    // Add steps
    workflow
      .step('step1', async (ctx: Context) => {
        return { message: 'Step 1 completed', timestamp: Date.now() };
      })
      .step('step2', async (ctx: Context) => {
        return {
          message: 'Step 2 completed',
          previous: ctx.last,
          timestamp: Date.now(),
        };
      })
      .step('step3', async (ctx: Context) => {
        return {
          message: 'Step 3 completed',
          allSteps: ctx.steps,
          timestamp: Date.now(),
        };
      });

    // Initialize and trigger
    await cronflow.start();

    const payload = {
      iteration,
      timestamp: Date.now(),
      data: Array.from({ length: 100 }, (_, i) => `item-${i}`),
    };

    const runId = await cronflow.trigger(
      `${TEST_WORKFLOW_ID}-${iteration}`,
      payload
    );

    const endTime = process.hrtime.bigint();
    const endMemory = getMemoryUsage();
    const duration = Number(endTime - startTime) / 1000000; // Convert to ms

    return {
      iteration,
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
  } catch (error) {
    const endTime = process.hrtime.bigint();
    const duration = Number(endTime - startTime) / 1000000;

    return {
      iteration,
      success: false,
      error: error instanceof Error ? error.message : String(error),
      duration,
    };
  } finally {
    await cronflow.stop();
  }
}

async function runPerformanceBenchmark() {
  console.log('🚀 Node-Cronflow Performance Benchmark');
  console.log('='.repeat(60));
  console.log(`📊 Running ${ITERATIONS} iterations...`);
  console.log(`⏱️  Start Time: ${new Date().toISOString()}`);
  console.log('─'.repeat(60));

  const results: any[] = [];
  const durations: number[] = [];
  const memoryDeltas: number[] = [];

  // Run iterations
  for (let i = 1; i <= ITERATIONS; i++) {
    console.log(`🔄 Running iteration ${i}/${ITERATIONS}...`);
    const result = await runSingleBenchmark(i);
    results.push(result);

    if (result.success) {
      durations.push(result.duration);
      if (result.memory) {
        memoryDeltas.push(result.memory.delta.heapUsed);
      }
    }

    // Small delay between iterations
    await new Promise(resolve => setTimeout(resolve, 100));
  }

  // Calculate statistics
  const durationStats = calculateStats(durations);
  const memoryStats = calculateStats(memoryDeltas);

  // Performance summary
  console.log('─'.repeat(60));
  console.log('📈 PERFORMANCE BENCHMARK RESULTS');
  console.log('─'.repeat(60));

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
  console.log(`   - Median Heap Delta: ${memoryStats.median.toFixed(2)} MB`);
  console.log(`   - Min Heap Delta: ${memoryStats.min.toFixed(2)} MB`);
  console.log(`   - Max Heap Delta: ${memoryStats.max.toFixed(2)} MB`);

  console.log('\n🚀 Performance Metrics:');
  console.log(
    `   - Average Steps/Second: ${(3 / (durationStats.mean / 1000)).toFixed(2)}`
  );
  console.log(
    `   - Average Step Time: ${formatDuration(durationStats.mean / 3)}`
  );
  console.log(
    `   - Throughput: ${(ITERATIONS / (durationStats.mean / 1000)).toFixed(2)} workflows/second`
  );

  console.log('\n📋 Success Rate:');
  const successfulRuns = results.filter(r => r.success).length;
  const successRate = (successfulRuns / ITERATIONS) * 100;
  console.log(
    `   - Successful: ${successfulRuns}/${ITERATIONS} (${successRate.toFixed(1)}%)`
  );

  if (successfulRuns < ITERATIONS) {
    console.log('\n❌ Failed Runs:');
    results
      .filter(r => !r.success)
      .forEach(r => {
        console.log(`   - Iteration ${r.iteration}: ${r.error}`);
      });
  }

  console.log('─'.repeat(60));
  console.log('✅ Benchmark completed');

  return {
    success: true,
    results,
    statistics: {
      duration: durationStats,
      memory: memoryStats,
      successRate,
      throughput: ITERATIONS / (durationStats.mean / 1000),
    },
  };
}

// Run the benchmark
runPerformanceBenchmark()
  .then(result => {
    if (result.success) {
      console.log('🎯 Benchmark completed successfully');
      process.exit(0);
    } else {
      console.log('💥 Benchmark failed');
      process.exit(1);
    }
  })
  .catch(error => {
    console.error('💥 Unexpected error:', error);
    process.exit(1);
  });

export { runPerformanceBenchmark };
