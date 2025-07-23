#!/usr/bin/env bun

/**
 * Simple Workflow Test with Performance Monitoring
 *
 * This example demonstrates a simple workflow with multiple steps
 * that each console log something, showing the step execution flow.
 * Includes performance monitoring for execution time and memory usage.
 */

import { cronflow } from '../sdk/src/index';
import { Context } from '../sdk/src/workflow/types';

// Test configuration
const TEST_WORKFLOW_ID = 'simple-test-workflow';

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
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60000) return `${(ms / 1000).toFixed(2)}s`;
  return `${(ms / 60000).toFixed(2)}m`;
}

async function runSimpleWorkflowTest() {
  // Performance monitoring start
  const startTime = process.hrtime.bigint();
  const startMemory = getMemoryUsage();

  console.log('🚀 Starting Simple Workflow Test with Performance Monitoring');
  console.log('📊 Initial Memory Usage:', startMemory);
  console.log('⏱️  Start Time:', new Date().toISOString());
  console.log('─'.repeat(60));

  try {
    // Create a simple workflow with fluent interface
    const workflow = cronflow.define({
      id: TEST_WORKFLOW_ID,
      name: 'Simple Test Workflow',
      description: 'A simple workflow to test step execution',
    });

    // Add steps using fluent interface (method chaining)
    workflow
      .step(
        'greet',
        async (ctx: Context) => {
          const stepStartTime = process.hrtime.bigint();
          const stepStartMemory = getMemoryUsage();

          console.log('👋 Step 1: Greeting step executed!');
          console.log('   - Payload received:', ctx.payload);
          console.log('   - Step name: greet');
          console.log('   - Run ID:', ctx.run.id);

          const result = {
            message: 'Hello from step 1!',
            timestamp: Date.now(),
            step_number: 1,
          };

          const stepEndTime = process.hrtime.bigint();
          const stepEndMemory = getMemoryUsage();
          const stepDuration = Number(stepEndTime - stepStartTime) / 1000000; // Convert to ms

          console.log(
            '   - Step execution time:',
            formatDuration(stepDuration)
          );
          console.log('   - Step memory delta:', {
            rss: stepEndMemory.rss - stepStartMemory.rss,
            heapUsed: stepEndMemory.heapUsed - stepStartMemory.heapUsed,
          });

          return result;
        },
        {
          timeout: 5000,
        }
      )
      .step(
        'process',
        async (ctx: Context) => {
          const stepStartTime = process.hrtime.bigint();
          const stepStartMemory = getMemoryUsage();

          console.log('⚙️  Step 2: Processing step executed!');
          console.log('   - Previous step result:', ctx.last);
          console.log('   - Payload:', ctx.payload);
          console.log('   - Step name: process');

          // Simulate some processing
          const processedData = {
            original: ctx.payload,
            processed: true,
            timestamp: Date.now(),
            step_number: 2,
          };

          console.log('   - Processed data:', processedData);

          const stepEndTime = process.hrtime.bigint();
          const stepEndMemory = getMemoryUsage();
          const stepDuration = Number(stepEndTime - stepStartTime) / 1000000; // Convert to ms

          console.log(
            '   - Step execution time:',
            formatDuration(stepDuration)
          );
          console.log('   - Step memory delta:', {
            rss: stepEndMemory.rss - stepStartMemory.rss,
            heapUsed: stepEndMemory.heapUsed - stepStartMemory.heapUsed,
          });

          return processedData;
        },
        {
          timeout: 5000,
        }
      )
      .step(
        'report',
        async (ctx: Context) => {
          const stepStartTime = process.hrtime.bigint();
          const stepStartMemory = getMemoryUsage();

          console.log('📊 Step 3: Report generation step executed!');
          console.log('   - Previous step result:', ctx.last);
          console.log('   - All previous steps:', ctx.steps);
          console.log('   - Step name: report');

          // Create a summary report
          const report = {
            summary: 'Workflow execution completed successfully',
            steps_completed: Object.keys(ctx.steps).length + 1,
            total_steps: 3,
            execution_time: Date.now(),
            step_number: 3,
            final_result: {
              greeting: ctx.steps.greet,
              processed_data: ctx.steps.process,
              report_generated: true,
            },
          };

          console.log('   - Generated report:', report);

          const stepEndTime = process.hrtime.bigint();
          const stepEndMemory = getMemoryUsage();
          const stepDuration = Number(stepEndTime - stepStartTime) / 1000000; // Convert to ms

          console.log(
            '   - Step execution time:',
            formatDuration(stepDuration)
          );
          console.log('   - Step memory delta:', {
            rss: stepEndMemory.rss - stepStartMemory.rss,
            heapUsed: stepEndMemory.heapUsed - stepStartMemory.heapUsed,
          });

          return report;
        },
        {
          timeout: 5000,
        }
      );

    // Initialize Cronflow (workflows are automatically registered)
    await cronflow.start();

    // Create a workflow run with some test data
    const payload = {
      user: 'test-user',
      data: {
        items: ['apple', 'banana', 'cherry'],
        count: 3,
        timestamp: Date.now(),
      },
      message: 'Hello from test payload!',
    };

    // Trigger the workflow - it will automatically execute all steps
    const runId = await cronflow.trigger(TEST_WORKFLOW_ID, payload);

    // Performance monitoring end
    const endTime = process.hrtime.bigint();
    const endMemory = getMemoryUsage();
    const totalDuration = Number(endTime - startTime) / 1000000; // Convert to ms

    // Performance summary
    console.log('─'.repeat(60));
    console.log('📈 PERFORMANCE SUMMARY');
    console.log('─'.repeat(60));
    console.log('⏱️  Total Execution Time:', formatDuration(totalDuration));
    console.log('📊 Memory Usage:');
    console.log('   - RSS (Resident Set Size):', endMemory.rss, 'MB');
    console.log('   - Heap Total:', endMemory.heapTotal, 'MB');
    console.log('   - Heap Used:', endMemory.heapUsed, 'MB');
    console.log('   - External:', endMemory.external, 'MB');
    console.log('📈 Memory Changes:');
    console.log('   - RSS Delta:', endMemory.rss - startMemory.rss, 'MB');
    console.log(
      '   - Heap Used Delta:',
      endMemory.heapUsed - startMemory.heapUsed,
      'MB'
    );
    console.log('🚀 Performance Metrics:');
    console.log(
      '   - Steps per second:',
      (3 / (totalDuration / 1000)).toFixed(2)
    );
    console.log('   - Average step time:', formatDuration(totalDuration / 3));
    console.log(
      '   - Memory per step:',
      Math.round((endMemory.heapUsed - startMemory.heapUsed) / 3),
      'MB'
    );

    return {
      success: true,
      run_id: runId,
      summary:
        'Simple workflow completed successfully with all steps executed automatically',
      performance: {
        total_duration_ms: totalDuration,
        memory_usage: endMemory,
        memory_delta: {
          rss: endMemory.rss - startMemory.rss,
          heapUsed: endMemory.heapUsed - startMemory.heapUsed,
        },
        steps_per_second: 3 / (totalDuration / 1000),
        average_step_time_ms: totalDuration / 3,
      },
    };
  } catch (error: any) {
    console.error('❌ Test failed:', error);
    console.error('Stack trace:', error.stack);
    return {
      success: false,
      error: error.message,
      stack: error.stack,
    };
  } finally {
    // Clean up
    await cronflow.stop();
  }
}

// Run the test
runSimpleWorkflowTest()
  .then(result => {
    if (result.success) {
      console.log('✅ Test completed successfully');
      process.exit(0);
    } else {
      console.log('❌ Test failed');
      process.exit(1);
    }
  })
  .catch(error => {
    console.error('💥 Unexpected error:', error);
    process.exit(1);
  });

export { runSimpleWorkflowTest };
