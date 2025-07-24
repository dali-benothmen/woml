import { define, start, trigger } from '../sdk/src/cronflow';

console.log('=== Simple If/Else Control Flow Test ===\n');

// Simple workflow with if/else condition
const simpleIfElseWorkflow = define({
  id: 'simple-if-else-test',
  name: 'Simple If/Else Test',
  description: 'Test basic if/else control flow with minimal steps',
})
  .step('check-amount', async ctx => {
    console.log('✅ Step 1: check-amount executed');
    console.log('   Payload amount:', ctx.payload.amount);
    return { amount: ctx.payload.amount, checked: true };
  })
  .if('is-high-value', ctx => {
    console.log('🔍 Evaluating condition: is-high-value');
    console.log('   Condition: ctx.last.amount > 50');
    console.log('   Last step amount:', ctx.last.amount);
    const result = ctx.last.amount > 50;
    console.log('   Condition result:', result);
    return result;
  })
  .step('process-high-value', async ctx => {
    console.log('✅ Step 2: process-high-value executed (IF branch)');
    console.log('   Processing high value amount:', ctx.last.amount);
    return { type: 'high-value', processed: true, amount: ctx.last.amount };
  })
  .else()
  .step('process-low-value', async ctx => {
    console.log('✅ Step 3: process-low-value executed (ELSE branch)');
    console.log('   Processing low value amount:', ctx.last.amount);
    return { type: 'low-value', processed: true, amount: ctx.last.amount };
  })
  .endIf()
  .step('final-step', async ctx => {
    console.log('✅ Step 4: final-step executed');
    console.log('   Previous step result:', ctx.last);
    return { final: true, summary: ctx.last };
  });

console.log('📋 Workflow Definition:');
console.log('Workflow ID:', simpleIfElseWorkflow.getId());
console.log('Workflow Name:', simpleIfElseWorkflow.getName());
console.log('Total Steps:', simpleIfElseWorkflow.getSteps().length);
console.log('\n');

// Test Case 1: High value (should execute IF branch)
console.log('🧪 Test Case 1: High Value (amount = 100)');
console.log(
  'Expected: process-high-value should execute, process-low-value should NOT execute\n'
);

const highValuePayload = { amount: 100 };
console.log('Payload:', JSON.stringify(highValuePayload, null, 2));
console.log('\n--- Execution Log ---');

// Start the workflow engine and execute
(async () => {
  try {
    await start();
    console.log('🚀 Workflow engine started');

    // Execute the workflow
    const runId = await trigger('simple-if-else-test', highValuePayload);
    console.log('📋 Workflow triggered with run ID:', runId);

    // Wait a moment for execution
    await new Promise(resolve => setTimeout(resolve, 1000));

    console.log('✅ Test Case 1 completed\n');
  } catch (error) {
    console.error('❌ Error during execution:', error);
  }
})();

// Test Case 2: Low value (should execute ELSE branch)
console.log('\n🧪 Test Case 2: Low Value (amount = 30)');
console.log(
  'Expected: process-low-value should execute, process-high-value should NOT execute\n'
);

const lowValuePayload = { amount: 30 };
console.log('Payload:', JSON.stringify(lowValuePayload, null, 2));
console.log('\n--- Execution Log ---');

// Execute the workflow for low value
(async () => {
  try {
    // Execute the workflow
    const runId = await trigger('simple-if-else-test', lowValuePayload);
    console.log('📋 Workflow triggered with run ID:', runId);

    // Wait a moment for execution
    await new Promise(resolve => setTimeout(resolve, 1000));

    console.log('✅ Test Case 2 completed');
  } catch (error) {
    console.error('❌ Error during execution:', error);
  }
})();
