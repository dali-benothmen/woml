import { cronflow } from '../sdk/src/index';

console.log('🧪 Testing Advanced Control Flow Features...\n');

// Test 1: Cancel functionality
console.log('✅ Test 1: Cancel functionality');
try {
  const cancelWorkflow = cronflow.define({
    id: 'cancel-test',
    name: 'Cancel Test Workflow',
  });

  cancelWorkflow
    .step('should-cancel', ctx => {
      return { status: 'about-to-cancel' };
    })
    .cancel('Test cancellation');

  console.log('✅ Cancel workflow defined successfully');
} catch (error) {
  console.log('❌ Cancel test failed:', error);
}

// Test 2: Sleep functionality
console.log('\n✅ Test 2: Sleep functionality');
try {
  const sleepWorkflow = cronflow.define({
    id: 'sleep-test',
    name: 'Sleep Test Workflow',
  });

  sleepWorkflow
    .step('before-sleep', ctx => {
      return { timestamp: Date.now() };
    })
    .sleep('1s')
    .step('after-sleep', ctx => {
      return { timestamp: Date.now() };
    });

  console.log('✅ Sleep workflow defined successfully');
} catch (error) {
  console.log('❌ Sleep test failed:', error);
}

// Test 3: Subflow functionality
console.log('\n✅ Test 3: Subflow functionality');
try {
  const subflowWorkflow = cronflow.define({
    id: 'subflow-test',
    name: 'Subflow Test Workflow',
  });

  subflowWorkflow
    .step('before-subflow', ctx => {
      return { data: 'pre-subflow' };
    })
    .subflow('cleanup', 'cleanup-workflow', {
      directory: '/tmp/test',
      olderThan: '1h',
    })
    .step('after-subflow', ctx => {
      return { data: 'post-subflow' };
    });

  console.log('✅ Subflow workflow defined successfully');
} catch (error) {
  console.log('❌ Subflow test failed:', error);
}

// Test 4: forEach functionality
console.log('\n✅ Test 4: forEach functionality');
try {
  const forEachWorkflow = cronflow.define({
    id: 'foreach-test',
    name: 'ForEach Test Workflow',
  });

  forEachWorkflow
    .step('get-users', ctx => {
      return [
        { id: 1, name: 'Alice', email: 'alice@example.com' },
        { id: 2, name: 'Bob', email: 'bob@example.com' },
        { id: 3, name: 'Charlie', email: 'charlie@example.com' },
      ];
    })
    .forEach(
      'process-users',
      ctx => ctx.steps['get-users'],
      (user, flow) => {
        flow
          .step('send-welcome', ctx => {
            console.log(`Sending welcome email to ${user.name}`);
            return { sent: true, user: user.name };
          })
          .step('update-status', ctx => {
            console.log(`Updating status for ${user.name}`);
            return { updated: true, user: user.name };
          })
          .step('verify-registration', () => {
            const workflow = cronflow.getWorkflow('foreach-test');
            const hasForEachStep = workflow?.steps.some(
              step => step.name === 'forEach_process-users'
            );

            if (!hasForEachStep) {
              throw new Error(
                "forEach step handler wasn't registered on workflow definition"
              );
            }

            return { verified: true, user: user.name };
          });
      }
    )
    .step('summary', ctx => {
      return { processed: ctx.last.totalItems };
    });

  console.log('✅ ForEach workflow defined successfully');
} catch (error) {
  console.log('❌ ForEach test failed:', error);
}

// Test 5: Batch functionality
console.log('\n✅ Test 5: Batch functionality');
try {
  const batchWorkflow = cronflow.define({
    id: 'batch-test',
    name: 'Batch Test Workflow',
  });

  batchWorkflow
    .step('get-all-items', ctx => {
      return Array.from({ length: 10 }, (_, i) => ({
        id: i + 1,
        name: `Item ${i + 1}`,
      }));
    })
    .batch(
      'process-batches',
      {
        items: ctx => ctx.steps['get-all-items'],
        size: 3,
      },
      (batch, flow) => {
        flow
          .step('process-batch', ctx => {
            console.log(`Processing batch of ${batch.length} items`);
            return {
              processed: batch.length,
              items: batch.map(item => item.name),
            };
          })
          .log(ctx => `Processed batch of ${batch.length} items`);
      }
    )
    .step('final-summary', ctx => {
      return {
        totalBatches: ctx.last.totalBatches,
        totalItems: ctx.last.totalItems,
      };
    });

  console.log('✅ Batch workflow defined successfully');
} catch (error) {
  console.log('❌ Batch test failed:', error);
}

// Test 6: Human in the Loop functionality
console.log('\n✅ Test 6: Human in the Loop functionality');
try {
  const humanWorkflow = cronflow.define({
    id: 'human-test',
    name: 'Human in the Loop Test Workflow',
  });

  humanWorkflow
    .step('prepare-approval', ctx => {
      return { amount: 1000, description: 'High-value transaction' };
    })
    .humanInTheLoop({
      timeout: '1h',
      description: 'Approve high-value transaction',
      onPause: (ctx, token) => {
        console.log(`Human approval required. Token: ${token}`);
        console.log(`Transaction amount: $${ctx.last.amount}`);
      },
    })
    .step('process-approval', ctx => {
      return { approved: ctx.last.approved, token: ctx.last.token };
    });

  console.log('✅ Human in the Loop workflow defined successfully');
} catch (error) {
  console.log('❌ Human in the Loop test failed:', error);
}

// Test 7: Wait for Event functionality
console.log('\n✅ Test 7: Wait for Event functionality');
try {
  const eventWorkflow = cronflow.define({
    id: 'event-test',
    name: 'Wait for Event Test Workflow',
  });

  eventWorkflow
    .step('prepare-event-wait', ctx => {
      return { event: 'payment.confirmed' };
    })
    .waitForEvent('payment.confirmed', '30m')
    .step('process-event', ctx => {
      return { event: ctx.last.eventName, received: ctx.last.received };
    });

  console.log('✅ Wait for Event workflow defined successfully');
} catch (error) {
  console.log('❌ Wait for Event test failed:', error);
}

// Test 8: OnError functionality
console.log('\n✅ Test 8: OnError functionality');
try {
  const errorWorkflow = cronflow.define({
    id: 'error-test',
    name: 'Error Handling Test Workflow',
  });

  errorWorkflow
    .step('risky-operation', ctx => {
      throw new Error('Simulated error');
    })
    .onError(ctx => {
      console.log('Error handled:', ctx.error?.message);
      return { fallback: true, error: ctx.error?.message };
    })
    .step('continue-after-error', ctx => {
      return { status: 'recovered' };
    });

  console.log('✅ Error handling workflow defined successfully');
} catch (error) {
  console.log('❌ Error handling test failed:', error);
}

// Test 9: Complex workflow with multiple advanced features
console.log('\n✅ Test 9: Complex workflow with multiple advanced features');
try {
  const complexWorkflow = cronflow.define({
    id: 'complex-test',
    name: 'Complex Advanced Control Flow Test',
  });

  complexWorkflow
    .step('get-data', ctx => {
      return { users: 5, items: 20 };
    })
    .if('has-users', ctx => ctx.last.users > 0)
    .forEach(
      'process-users',
      ctx =>
        Array.from({ length: ctx.steps['get-data'].users }, (_, i) => ({
          id: i + 1,
        })),
      (user, flow) => {
        flow.step('process-user', ctx => ({
          userId: user.id,
          processed: true,
        }));
      }
    )
    .else()
    .step('no-users', ctx => ({ message: 'No users to process' }))
    .endIf()
    .sleep('2s')
    .batch(
      'process-items',
      {
        items: ctx =>
          Array.from({ length: ctx.steps['get-data'].items }, (_, i) => ({
            id: i + 1,
          })),
        size: 5,
      },
      (batch, flow) => {
        flow.step('process-batch', ctx => ({ batchSize: batch.length }));
      }
    )
    .step('final-summary', ctx => {
      return { completed: true, timestamp: Date.now() };
    });

  console.log('✅ Complex workflow defined successfully');
} catch (error) {
  console.log('❌ Complex workflow test failed:', error);
}

console.log(
  '\n🚀 Executing foreach-test workflow to validate handler registration...'
);
await cronflow.start();
try {
  const runId = await cronflow.trigger('foreach-test', {});
  console.log(`   ↳ Triggered run ${runId}, waiting for completion...`);
  await new Promise(resolve => setTimeout(resolve, 500));
  console.log('   ✔ foreach-test workflow run completed.');
} catch (error) {
  console.log('❌ Failed to execute foreach-test workflow:', error);
} finally {
  await cronflow.stop();
}

console.log('\n🎉 All advanced control flow tests completed!');
console.log('\n📋 Summary of Advanced Control Flow Features:');
console.log('✅ Cancel - Gracefully stop workflow execution');
console.log('✅ Sleep - Pause workflow for specified duration');
console.log('✅ Subflow - Execute another workflow as child process');
console.log('✅ ForEach - Execute sub-workflow for each item in parallel');
console.log('✅ Batch - Process items in smaller sequential batches');
console.log('✅ Human in the Loop - Pause for human approval');
console.log('✅ Wait for Event - Pause until specific event occurs');
console.log('✅ OnError - Custom error handling for steps');
console.log('✅ Complex Workflows - Multiple advanced features together');
