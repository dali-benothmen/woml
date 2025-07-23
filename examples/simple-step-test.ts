#!/usr/bin/env bun

import { cronflow } from '../sdk/src/index';

// Create a very simple workflow
const simpleWorkflow = cronflow.define({
  id: 'simple-step-test',
  name: 'Simple Step Test',
});

simpleWorkflow
  .step('hello', async ctx => {
    console.log('Hello from step!');
    console.log('Payload:', ctx.payload);
    return { message: 'Hello world!' };
  })
  .action('goodbye', ctx => {
    console.log('Goodbye from action!');
    console.log('Previous step result:', ctx.last);
  });

async function test() {
  console.log('🚀 Starting simple test...');

  await cronflow.start();

  console.log('📤 Triggering workflow...');
  await cronflow.trigger('simple-step-test', { test: 'data' });

  await new Promise(resolve => setTimeout(resolve, 1000));

  console.log('🛑 Stopping...');
  await cronflow.stop();

  console.log('✅ Done!');
}

test().catch(console.error);
