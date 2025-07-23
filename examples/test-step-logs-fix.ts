#!/usr/bin/env bun

import { cronflow } from '../sdk/src/index';

// Create a simple workflow with clear logging
const testWorkflow = cronflow.define({
  id: 'test-step-logs-fix',
  name: 'Test Step Logs Fix',
});

testWorkflow
  .step('step-1', async ctx => {
    console.log('🔍 STEP 1: This log should now appear!');
    console.log('🔍 STEP 1: Payload:', ctx.payload);
    return { step1: 'completed' };
  })
  .step('step-2', async ctx => {
    console.log('🔍 STEP 2: This log should also appear!');
    console.log('🔍 STEP 2: Previous step result:', ctx.last);
    return { step2: 'completed', step1Result: ctx.last };
  })
  .action('final-action', ctx => {
    console.log('🔍 ACTION: Final action log!');
    console.log('🔍 ACTION: All steps completed:', Object.keys(ctx.steps));
  });

async function testStepLogsFix() {
  console.log('🚀 Testing step logs fix...');

  await cronflow.start();

  console.log('📤 Triggering workflow...');
  const runId = await cronflow.trigger('test-step-logs-fix', {
    message: 'Hello from test!',
    test: true,
  });

  console.log('⏳ Waiting for execution...');
  await new Promise(resolve => setTimeout(resolve, 2000));

  console.log('🛑 Stopping...');
  await cronflow.stop();

  console.log('✅ Test completed!');
}

testStepLogsFix().catch(console.error);
