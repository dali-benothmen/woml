#!/usr/bin/env bun

/**
 * Benchmark Demo
 *
 * Demonstrates the new cronflow.benchmark() method
 */

import { cronflow } from '../sdk/src/index';

async function runBenchmarkDemo() {
  console.log('🎯 Cronflow Benchmark Demo');
  console.log('='.repeat(50));

  // Run a quick benchmark with default settings
  console.log('\n📊 Running quick benchmark (5 iterations, 3 steps each)...');
  const quickResult = await cronflow.benchmark({
    iterations: 5,
    stepsPerWorkflow: 3,
    payloadSize: 50,
    verbose: true,
  });

  console.log('\n📈 Quick Benchmark Summary:');
  console.log(
    `   - Average execution time: ${quickResult.statistics.duration.mean.toFixed(2)}ms`
  );
  console.log(
    `   - Throughput: ${quickResult.statistics.throughput.toFixed(2)} workflows/second`
  );
  console.log(
    `   - Success rate: ${quickResult.statistics.successRate.toFixed(1)}%`
  );

  // Run a more intensive benchmark
  console.log(
    '\n📊 Running intensive benchmark (10 iterations, 5 steps each)...'
  );
  const intensiveResult = await cronflow.benchmark({
    iterations: 10,
    stepsPerWorkflow: 5,
    payloadSize: 100,
    delayBetweenRuns: 50,
    verbose: true,
  });

  console.log('\n📈 Intensive Benchmark Summary:');
  console.log(
    `   - Average execution time: ${intensiveResult.statistics.duration.mean.toFixed(2)}ms`
  );
  console.log(
    `   - Throughput: ${intensiveResult.statistics.throughput.toFixed(2)} workflows/second`
  );
  console.log(
    `   - Steps per second: ${intensiveResult.statistics.stepsPerSecond.toFixed(2)}`
  );
  console.log(
    `   - Success rate: ${intensiveResult.statistics.successRate.toFixed(1)}%`
  );

  // Run a silent benchmark (no console output)
  console.log('\n📊 Running silent benchmark...');
  const silentResult = await cronflow.benchmark({
    iterations: 3,
    stepsPerWorkflow: 2,
    payloadSize: 25,
    verbose: false,
  });

  console.log('\n📈 Silent Benchmark Results:');
  console.log(
    `   - Mean: ${silentResult.statistics.duration.mean.toFixed(2)}ms`
  );
  console.log(
    `   - Median: ${silentResult.statistics.duration.median.toFixed(2)}ms`
  );
  console.log(`   - Min: ${silentResult.statistics.duration.min.toFixed(2)}ms`);
  console.log(`   - Max: ${silentResult.statistics.duration.max.toFixed(2)}ms`);
  console.log(
    `   - Throughput: ${silentResult.statistics.throughput.toFixed(2)} workflows/second`
  );

  return {
    success: true,
    quick: quickResult,
    intensive: intensiveResult,
    silent: silentResult,
  };
}

// Run the demo
runBenchmarkDemo()
  .then(result => {
    if (result.success) {
      console.log('\n✅ Benchmark demo completed successfully');
      process.exit(0);
    } else {
      console.log('\n❌ Benchmark demo failed');
      process.exit(1);
    }
  })
  .catch(error => {
    console.error('💥 Unexpected error:', error);
    process.exit(1);
  });

export { runBenchmarkDemo };
