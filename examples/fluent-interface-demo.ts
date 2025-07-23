#!/usr/bin/env bun

/**
 * Fluent Interface Demo
 *
 * This example demonstrates the elegant fluent interface API
 * with method chaining for workflow definition.
 */

import { cronflow } from '../sdk/src/index';
import { Context } from '../sdk/src/workflow/types';

// Test configuration
const TEST_WORKFLOW_ID = 'fluent-interface-demo';

async function runFluentInterfaceDemo() {
  try {
    // Create a workflow using the elegant fluent interface
    const workflow = cronflow.define({
      id: TEST_WORKFLOW_ID,
      name: 'Fluent Interface Demo',
      description: 'Demonstrating the elegant fluent interface API',
    });

    // Define the workflow using method chaining
    workflow
      // Step 1: Data validation
      .step('validate', async (ctx: Context) => {
        console.log('🔍 Step 1: Data validation executed!');
        console.log('   - Validating payload:', ctx.payload);

        if (!ctx.payload.user) {
          throw new Error('User is required');
        }

        return {
          validated: true,
          user: ctx.payload.user,
          timestamp: Date.now(),
        };
      })

      // Step 2: Data processing
      .step('process', async (ctx: Context) => {
        console.log('⚙️  Step 2: Data processing executed!');
        console.log('   - Previous step result:', ctx.last);

        const processedData = {
          ...ctx.last,
          processed: true,
          items: ctx.payload.data?.items || [],
          count: ctx.payload.data?.count || 0,
        };

        return processedData;
      })

      // Step 3: Generate report
      .step('report', async (ctx: Context) => {
        console.log('📊 Step 3: Report generation executed!');
        console.log('   - All previous steps:', ctx.steps);

        const report = {
          summary: 'Fluent interface demo completed',
          steps: Object.keys(ctx.steps),
          final_data: ctx.steps.process,
          generated_at: Date.now(),
        };

        return report;
      });

    // Initialize Cronflow (workflows are automatically registered)
    await cronflow.start();

    // Create a workflow run
    const payload = {
      user: 'fluent-demo-user',
      data: {
        items: ['item1', 'item2', 'item3'],
        count: 3,
      },
      message: 'Testing fluent interface!',
    };

    // Trigger the workflow - it will automatically execute all steps
    const runId = await cronflow.trigger(TEST_WORKFLOW_ID, payload);

    return {
      success: true,
      run_id: runId,
      summary:
        'Fluent interface demo completed successfully with all steps executed automatically',
    };
  } catch (error: any) {
    console.error('❌ Demo failed:', error);
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

// Run the demo
runFluentInterfaceDemo()
  .then(result => {
    if (result.success) {
      process.exit(0);
    } else {
      process.exit(1);
    }
  })
  .catch(error => {
    console.error('💥 Unexpected error:', error);
    process.exit(1);
  });

export { runFluentInterfaceDemo };
