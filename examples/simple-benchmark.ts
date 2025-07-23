#!/usr/bin/env bun

/**
 * Simple Benchmark Example
 *
 * Shows how to use the cronflow.benchmark() method for performance testing
 */

import { cronflow } from '../sdk/src/index';

async function runSimpleBenchmark() {
  console.log('🚀 Simple Cronflow Benchmark');
  console.log('='.repeat(40));

  // Run a simple benchmark with default settings
  const result = await cronflow.benchmark({
    iterations: 5,
    stepsPerWorkflow: 3,
    payloadSize: 50,
    verbose: true,
  });

  console.log('\n📊 Benchmark Summary:');
  console.log(`✅ Success Rate: ${result.statistics.successRate.toFixed(1)}%`);
  console.log(
    `⚡ Average Time: ${result.statistics.duration.mean.toFixed(2)}ms`
  );
  console.log(
    `🚀 Throughput: ${result.statistics.throughput.toFixed(2)} workflows/second`
  );
  console.log(
    `🔧 Steps/Second: ${result.statistics.stepsPerSecond.toFixed(2)}`
  );

  return result;
}

// Run the benchmark
runSimpleBenchmark()
  .then(result => {
    console.log('\n✅ Benchmark completed successfully');
    process.exit(0);
  })
  .catch(error => {
    console.error('❌ Benchmark failed:', error);
    process.exit(1);
  });

export { runSimpleBenchmark };
