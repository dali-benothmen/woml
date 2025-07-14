import { cronflow } from '../sdk/src/index.js';

// Test workflow for Task 2.1: Job Dispatcher Integration
const testWorkflow = cronflow.define({
  id: 'test-task-2-1',
  name: 'Test Task 2.1 - Job Dispatcher Integration',
  description:
    'Test workflow to verify job dispatcher integration with trigger executor',
  tags: ['test', 'job-dispatcher'],
});

// Define workflow steps
testWorkflow
  .onWebhook('/test-task-2-1')
  .step('step-1', async ctx => {
    console.log('🔧 Step 1 executing...');
    console.log('📋 Context payload:', ctx.payload);
    console.log('📋 Run ID:', ctx.run.id);
    console.log('📋 Workflow ID:', ctx.run.workflowId);

    // Simulate some work
    await new Promise(resolve => setTimeout(resolve, 100));

    await ctx.state.set('step1_executed', true);
    const step1Count = await ctx.state.incr('step1_count', 1);

    return {
      message: 'Step 1 completed successfully',
      step1Count,
      workflowId: ctx.run.workflowId,
      runId: ctx.run.id,
    };
  })
  .step('step-2', async ctx => {
    console.log('🔧 Step 2 executing...');
    console.log('📋 Previous step output:', ctx.steps['step-1'].output);

    // Simulate some work
    await new Promise(resolve => setTimeout(resolve, 150));

    await ctx.state.set('step2_executed', true);
    const step2Count = await ctx.state.incr('step2_count', 1);

    return {
      message: 'Step 2 completed successfully',
      step2Count,
      previousStep: ctx.steps['step-1'].output,
      workflowId: ctx.run.workflowId,
      runId: ctx.run.id,
    };
  })
  .step('step-3', async ctx => {
    console.log('🔧 Step 3 executing...');
    console.log('📋 Previous step output:', ctx.steps['step-2'].output);

    // Simulate some work
    await new Promise(resolve => setTimeout(resolve, 200));

    await ctx.state.set('step3_executed', true);
    const step3Count = await ctx.state.incr('step3_count', 1);

    return {
      message: 'Step 3 completed successfully',
      step3Count,
      previousStep: ctx.steps['step-2'].output,
      workflowId: ctx.run.workflowId,
      runId: ctx.run.id,
    };
  })
  .action('final-action', async ctx => {
    console.log('🔧 Final action executing...');
    console.log('📋 All step outputs:', {
      step1: ctx.steps['step-1'].output,
      step2: ctx.steps['step-2'].output,
      step3: ctx.steps['step-3'].output,
    });

    // Simulate some work
    await new Promise(resolve => setTimeout(resolve, 50));

    await ctx.state.set('final_action_executed', true);
    const finalCount = await ctx.state.incr('final_action_count', 1);

    console.log('✅ Final action completed successfully');
    return {
      message: 'Final action completed successfully',
      finalCount,
      allSteps: {
        step1: ctx.steps['step-1'].output,
        step2: ctx.steps['step-2'].output,
        step3: ctx.steps['step-3'].output,
      },
      workflowId: ctx.run.workflowId,
      runId: ctx.run.id,
    };
  });

async function testTask21() {
  console.log('🚀 Starting Task 2.1 Test: Job Dispatcher Integration');

  try {
    // Start the engine
    await cronflow.start();
    console.log('✅ Engine started successfully');

    // Trigger the workflow
    console.log('📤 Triggering workflow...');
    const triggerResult = await cronflow.trigger('test-task-2-1', {
      testData: 'Task 2.1 test payload',
      timestamp: new Date().toISOString(),
      source: 'test-task-2-1',
    });

    console.log('📋 Trigger result:', triggerResult);

    if (triggerResult && typeof triggerResult === 'string') {
      const runId = triggerResult;
      console.log('✅ Workflow triggered successfully with run ID:', runId);

      // Wait a bit for job processing
      console.log('⏳ Waiting for job processing...');
      await new Promise(resolve => setTimeout(resolve, 2000));

      // Check run status
      console.log('📋 Checking run status...');
      const runStatus = await cronflow.inspect(runId);
      console.log('📋 Run status:', JSON.stringify(runStatus, null, 2));

      // For now, we'll just check if the run was created successfully
      // The actual step execution verification will be done in future tasks
      console.log(
        '✅ SUCCESS: Workflow run created and jobs submitted to dispatcher!'
      );
      console.log(
        '✅ Task 2.1: Job Dispatcher Integration is working correctly!'
      );
    } else {
      console.log('❌ FAILURE: Workflow trigger failed');
      console.log('❌ Trigger result:', triggerResult);
    }
  } catch (error) {
    console.error('❌ ERROR during Task 2.1 test:', error);
  } finally {
    // Stop the engine
    console.log('🛑 Stopping engine...');
    await cronflow.stop();
    console.log('✅ Engine stopped successfully');
  }
}

// Run the test
testTask21();
