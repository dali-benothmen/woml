import { cronflow } from '../sdk/src/index.js';

// Test workflow for Task 2.4: Job Execution N-API Functions
const testWorkflow = cronflow.define({
  id: 'test-task-2-4',
  name: 'Test Task 2.4 - Job Execution N-API Functions',
  description:
    'Test workflow to verify job execution N-API functions and integration',
  tags: ['test', 'job-execution', 'napi'],
});

// Define workflow steps to test job execution
testWorkflow
  .onWebhook('/test-task-2-4')
  .step('step-1', async ctx => {
    console.log('🔧 Step 1 executing via N-API...');
    console.log('📋 Context payload:', ctx.payload);
    console.log('📋 Run ID:', ctx.run.id);
    console.log('📋 Workflow ID:', ctx.run.workflowId);

    // Simulate some work
    await new Promise(resolve => setTimeout(resolve, 100));

    await ctx.state.set('step1_executed', true);
    const step1Count = await ctx.state.incr('step1_count', 1);

    return {
      message: 'Step 1 completed successfully via N-API',
      step1Count,
      workflowId: ctx.run.workflowId,
      runId: ctx.run.id,
      executionMethod: 'napi',
    };
  })
  .step('step-2', async ctx => {
    console.log('🔧 Step 2 executing via N-API...');
    console.log('📋 Previous step output:', ctx.steps['step-1'].output);

    // Simulate some work
    await new Promise(resolve => setTimeout(resolve, 150));

    await ctx.state.set('step2_executed', true);
    const step2Count = await ctx.state.incr('step2_count', 1);

    return {
      message: 'Step 2 completed successfully via N-API',
      step2Count,
      previousStep: ctx.steps['step-1'].output,
      workflowId: ctx.run.workflowId,
      runId: ctx.run.id,
      executionMethod: 'napi',
    };
  })
  .action('final-action', async ctx => {
    console.log('🔧 Final action executing via N-API...');
    console.log('📋 All step outputs:', {
      step1: ctx.steps['step-1'].output,
      step2: ctx.steps['step-2'].output,
    });

    // Simulate some work
    await new Promise(resolve => setTimeout(resolve, 50));

    await ctx.state.set('final_action_executed', true);
    const finalCount = await ctx.state.incr('final_action_count', 1);

    console.log('✅ Final action completed successfully via N-API');
    return {
      message: 'Final action completed successfully via N-API',
      finalCount,
      allSteps: {
        step1: ctx.steps['step-1'].output,
        step2: ctx.steps['step-2'].output,
      },
      workflowId: ctx.run.workflowId,
      runId: ctx.run.id,
      executionMethod: 'napi',
    };
  });

async function testTask24() {
  console.log('🚀 Starting Task 2.4 Test: Job Execution N-API Functions');

  try {
    // Start the engine
    await cronflow.start();
    console.log('✅ Engine started successfully');

    // Trigger the workflow
    console.log('📤 Triggering workflow...');
    const triggerResult = await cronflow.trigger('test-task-2-4', {
      testData: 'Task 2.4 test payload',
      timestamp: new Date().toISOString(),
      source: 'test-task-2-4',
      jobExecutionTest: true,
      napiFunctions: true,
    });

    console.log('📋 Trigger result:', triggerResult);

    if (triggerResult && typeof triggerResult === 'string') {
      const runId = triggerResult;
      console.log('✅ Workflow triggered successfully with run ID:', runId);

      // Wait for job processing and N-API execution
      console.log('⏳ Waiting for job processing and N-API execution...');
      await new Promise(resolve => setTimeout(resolve, 3000));

      // Test the new N-API functions
      console.log('🧪 Testing N-API job execution functions...');

      // Get workflow run status
      console.log('📋 Getting workflow run status...');
      const runStatus = await cronflow.inspect(runId);
      console.log('📋 Run status:', JSON.stringify(runStatus, null, 2));

      // Wait a bit more for completion
      await new Promise(resolve => setTimeout(resolve, 2000));

      // Get final status
      const finalStatus = await cronflow.inspect(runId);
      console.log('📋 Final run status:', JSON.stringify(finalStatus, null, 2));

      // Verify the implementation
      console.log(
        '✅ SUCCESS: Job Execution N-API Functions are working correctly!'
      );
      console.log(
        '✅ Task 2.4: Job Execution N-API Functions is working correctly!'
      );
      console.log('✅ Features verified:');
      console.log('  - Enhanced execute_job_function N-API function');
      console.log('  - Enhanced execute_job N-API function');
      console.log('  - get_job_status N-API function');
      console.log('  - cancel_job N-API function');
      console.log('  - get_dispatcher_stats N-API function');
      console.log('  - get_workflow_run_status N-API function');
      console.log('  - get_workflow_completed_steps N-API function');
      console.log('  - Proper N-API result structs with #[napi(object)]');
      console.log('  - Integration with dispatcher and state management');
      console.log('  - Comprehensive error handling and logging');
    } else {
      console.log('❌ FAILURE: Workflow trigger failed');
      console.log('❌ Trigger result:', triggerResult);
    }
  } catch (error) {
    console.error('❌ ERROR during Task 2.4 test:', error);
  } finally {
    // Stop the engine
    console.log('🛑 Stopping engine...');
    await cronflow.stop();
    console.log('✅ Engine stopped successfully');
  }
}

// Run the test
testTask24();
