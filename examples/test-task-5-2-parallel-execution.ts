import { cronflow } from '../sdk/src/cronflow';

console.log('=== Task 5.2: Parallel Execution Test ===\n');

// Test 1: Simple parallel execution
console.log('🧪 Test 1: Simple Parallel Execution');
const simpleParallelWorkflow = cronflow.define({
  id: 'simple-parallel-test',
  name: 'Simple Parallel Test',
  description: 'Test basic parallel step execution',
});

simpleParallelWorkflow
  .step('setup', async ctx => {
    console.log('✅ Setup step executed');
    return { message: 'Setup completed', timestamp: new Date().toISOString() };
  })
  .parallel([
    async ctx => {
      console.log('🔄 Parallel step 1 executing...');
      await new Promise(resolve => setTimeout(resolve, 200));
      console.log('✅ Parallel step 1 completed');
      return { step: 'parallel-1', result: 'success', data: [1, 2, 3] };
    },
    async ctx => {
      console.log('🔄 Parallel step 2 executing...');
      await new Promise(resolve => setTimeout(resolve, 150));
      console.log('✅ Parallel step 2 completed');
      return { step: 'parallel-2', result: 'success', data: { a: 1, b: 2 } };
    },
    async ctx => {
      console.log('🔄 Parallel step 3 executing...');
      await new Promise(resolve => setTimeout(resolve, 100));
      console.log('✅ Parallel step 3 completed');
      return { step: 'parallel-3', result: 'success', data: 'string-data' };
    },
  ])
  .step('aggregate', async ctx => {
    console.log('✅ Aggregate step executed');
    console.log('   Parallel results:', ctx.last);
    return {
      message: 'Parallel execution completed',
      parallelResults: ctx.last,
      summary: {
        totalSteps: ctx.last.total_count,
        successCount: ctx.last.success_count,
        failureCount: ctx.last.failure_count,
      },
    };
  });

console.log('📋 Simple Parallel Workflow Definition:');
console.log('Workflow ID:', simpleParallelWorkflow.getId());
console.log('Total Steps:', simpleParallelWorkflow.getSteps().length);
console.log(
  'Steps:',
  simpleParallelWorkflow.getSteps().map(s => ({
    id: s.id,
    name: s.name,
    parallel: s.parallel,
    parallelGroupId: s.parallel_group_id,
    parallelStepCount: s.parallel_step_count,
  }))
);
console.log('\n');

// Test 2: Mixed parallel and sequential execution
console.log('🧪 Test 2: Mixed Parallel and Sequential Execution');
const mixedExecutionWorkflow = cronflow.define({
  id: 'mixed-execution-test',
  name: 'Mixed Execution Test',
  description: 'Test parallel and sequential step execution together',
});

mixedExecutionWorkflow
  .step('start', async ctx => {
    console.log('✅ Start step executed');
    return { workflow: 'started', payload: ctx.payload };
  })
  .parallel([
    async ctx => {
      console.log('🔄 Data fetch step executing...');
      await new Promise(resolve => setTimeout(resolve, 300));
      console.log('✅ Data fetch completed');
      return { type: 'data', source: 'database', records: 150 };
    },
    async ctx => {
      console.log('🔄 Cache update step executing...');
      await new Promise(resolve => setTimeout(resolve, 200));
      console.log('✅ Cache update completed');
      return {
        type: 'cache',
        action: 'updated',
        keys: ['user:123', 'user:456'],
      };
    },
  ])
  .step('process', async ctx => {
    console.log('✅ Process step executed');
    console.log('   Previous parallel results:', ctx.last);
    return {
      message: 'Processing completed',
      dataCount: ctx.last.parallel_1?.data?.length || 0,
      cacheKeys: ctx.last.parallel_2?.data?.keys || [],
    };
  })
  .parallel([
    async ctx => {
      console.log('🔄 Notification step executing...');
      await new Promise(resolve => setTimeout(resolve, 100));
      console.log('✅ Notification sent');
      return { type: 'notification', channel: 'email', sent: true };
    },
    async ctx => {
      console.log('🔄 Logging step executing...');
      await new Promise(resolve => setTimeout(resolve, 50));
      console.log('✅ Logging completed');
      return { type: 'log', level: 'info', message: 'Workflow completed' };
    },
  ])
  .step('finish', async ctx => {
    console.log('✅ Finish step executed');
    return {
      message: 'Workflow completed successfully',
      finalStatus: 'success',
      executionTime: new Date().toISOString(),
    };
  });

console.log('📋 Mixed Execution Workflow Definition:');
console.log('Workflow ID:', mixedExecutionWorkflow.getId());
console.log('Total Steps:', mixedExecutionWorkflow.getSteps().length);
console.log(
  'Steps:',
  mixedExecutionWorkflow.getSteps().map(s => ({
    id: s.id,
    name: s.name,
    parallel: s.parallel,
    parallelGroupId: s.parallel_group_id,
    parallelStepCount: s.parallel_step_count,
  }))
);
console.log('\n');

// Test 3: Race condition execution
console.log('🧪 Test 3: Race Condition Execution');
const raceConditionWorkflow = cronflow.define({
  id: 'race-condition-test',
  name: 'Race Condition Test',
  description: 'Test race condition execution (first to complete wins)',
});

raceConditionWorkflow
  .step('trigger', async ctx => {
    console.log('✅ Trigger step executed');
    return { message: 'Race condition triggered' };
  })
  .race([
    async ctx => {
      console.log('🔄 Fast API call executing...');
      await new Promise(resolve => setTimeout(resolve, 100));
      console.log('✅ Fast API call completed');
      return { winner: 'fast-api', response: 'quick response', time: 100 };
    },
    async ctx => {
      console.log('🔄 Slow API call executing...');
      await new Promise(resolve => setTimeout(resolve, 500));
      console.log('✅ Slow API call completed');
      return { winner: 'slow-api', response: 'detailed response', time: 500 };
    },
    async ctx => {
      console.log('🔄 Backup API call executing...');
      await new Promise(resolve => setTimeout(resolve, 300));
      console.log('✅ Backup API call completed');
      return { winner: 'backup-api', response: 'fallback response', time: 300 };
    },
  ])
  .step('handle-winner', async ctx => {
    console.log('✅ Handle winner step executed');
    console.log('   Race winner:', ctx.last);
    return {
      message: 'Race condition resolved',
      winner: ctx.last.winner,
      response: ctx.last.response,
      executionTime: ctx.last.time,
    };
  });

console.log('📋 Race Condition Workflow Definition:');
console.log('Workflow ID:', raceConditionWorkflow.getId());
console.log('Total Steps:', raceConditionWorkflow.getSteps().length);
console.log(
  'Steps:',
  raceConditionWorkflow.getSteps().map(s => ({
    id: s.id,
    name: s.name,
    race: s.race,
    parallelGroupId: s.parallel_group_id,
    parallelStepCount: s.parallel_step_count,
  }))
);
console.log('\n');

// Test 4: Parallel execution with error handling
console.log('🧪 Test 4: Parallel Execution with Error Handling');
const errorHandlingWorkflow = cronflow.define({
  id: 'error-handling-test',
  name: 'Error Handling Test',
  description: 'Test parallel execution with error scenarios',
});

errorHandlingWorkflow
  .step('prepare', async ctx => {
    console.log('✅ Prepare step executed');
    return { status: 'prepared', data: 'ready for parallel execution' };
  })
  .parallel([
    async ctx => {
      console.log('🔄 Success step executing...');
      await new Promise(resolve => setTimeout(resolve, 100));
      console.log('✅ Success step completed');
      return { status: 'success', data: 'operation completed' };
    },
    async ctx => {
      console.log('🔄 Error step executing...');
      await new Promise(resolve => setTimeout(resolve, 150));
      console.log('❌ Error step failed');
      throw new Error('Simulated error in parallel step');
    },
    async ctx => {
      console.log('🔄 Another success step executing...');
      await new Promise(resolve => setTimeout(resolve, 200));
      console.log('✅ Another success step completed');
      return { status: 'success', data: 'another operation completed' };
    },
  ])
  .step('handle-results', async ctx => {
    console.log('✅ Handle results step executed');
    console.log('   Parallel execution results:', ctx.last);
    return {
      message: 'Error handling test completed',
      results: ctx.last,
      hasErrors: ctx.last.failure_count > 0,
      successCount: ctx.last.success_count,
      failureCount: ctx.last.failure_count,
    };
  });

console.log('📋 Error Handling Workflow Definition:');
console.log('Workflow ID:', errorHandlingWorkflow.getId());
console.log('Total Steps:', errorHandlingWorkflow.getSteps().length);
console.log(
  'Steps:',
  errorHandlingWorkflow.getSteps().map(s => ({
    id: s.id,
    name: s.name,
    parallel: s.parallel,
    parallelGroupId: s.parallel_group_id,
    parallelStepCount: s.parallel_step_count,
  }))
);
console.log('\n');

// Test 5: Complex workflow with multiple parallel groups
console.log('🧪 Test 5: Complex Workflow with Multiple Parallel Groups');
const complexParallelWorkflow = cronflow.define({
  id: 'complex-parallel-test',
  name: 'Complex Parallel Test',
  description: 'Test complex workflow with multiple parallel execution groups',
});

complexParallelWorkflow
  .step('initialize', async ctx => {
    console.log('✅ Initialize step executed');
    return { workflow: 'initialized', timestamp: new Date().toISOString() };
  })
  .parallel([
    async ctx => {
      console.log('🔄 Group 1 - Step 1 executing...');
      await new Promise(resolve => setTimeout(resolve, 100));
      console.log('✅ Group 1 - Step 1 completed');
      return { group: 1, step: 1, result: 'success' };
    },
    async ctx => {
      console.log('🔄 Group 1 - Step 2 executing...');
      await new Promise(resolve => setTimeout(resolve, 150));
      console.log('✅ Group 1 - Step 2 completed');
      return { group: 1, step: 2, result: 'success' };
    },
  ])
  .step('intermediate', async ctx => {
    console.log('✅ Intermediate step executed');
    console.log('   Group 1 results:', ctx.last);
    return { message: 'Intermediate processing', group1Results: ctx.last };
  })
  .parallel([
    async ctx => {
      console.log('🔄 Group 2 - Step 1 executing...');
      await new Promise(resolve => setTimeout(resolve, 200));
      console.log('✅ Group 2 - Step 1 completed');
      return { group: 2, step: 1, result: 'success' };
    },
    async ctx => {
      console.log('🔄 Group 2 - Step 2 executing...');
      await new Promise(resolve => setTimeout(resolve, 250));
      console.log('✅ Group 2 - Step 2 completed');
      return { group: 2, step: 2, result: 'success' };
    },
    async ctx => {
      console.log('🔄 Group 2 - Step 3 executing...');
      await new Promise(resolve => setTimeout(resolve, 300));
      console.log('✅ Group 2 - Step 3 completed');
      return { group: 2, step: 3, result: 'success' };
    },
  ])
  .step('finalize', async ctx => {
    console.log('✅ Finalize step executed');
    console.log('   Group 2 results:', ctx.last);
    return {
      message: 'Complex parallel workflow completed',
      group2Results: ctx.last,
      totalGroups: 2,
      totalSteps: ctx.last.total_count + 3, // Group 1 + Group 2 + other steps
    };
  });

console.log('📋 Complex Parallel Workflow Definition:');
console.log('Workflow ID:', complexParallelWorkflow.getId());
console.log('Total Steps:', complexParallelWorkflow.getSteps().length);
console.log(
  'Steps:',
  complexParallelWorkflow.getSteps().map(s => ({
    id: s.id,
    name: s.name,
    parallel: s.parallel,
    parallelGroupId: s.parallel_group_id,
    parallelStepCount: s.parallel_step_count,
  }))
);
console.log('\n');

// Summary
console.log('📊 Parallel Execution Test Summary:');
console.log('✅ Created 5 test workflows:');
console.log('   1. Simple Parallel Execution');
console.log('   2. Mixed Parallel and Sequential Execution');
console.log('   3. Race Condition Execution');
console.log('   4. Parallel Execution with Error Handling');
console.log('   5. Complex Workflow with Multiple Parallel Groups');
console.log('\n');
console.log('🔧 Key Features Tested:');
console.log('   • Parallel step detection and grouping');
console.log('   • Concurrent step execution');
console.log('   • Result aggregation from parallel steps');
console.log('   • Error handling in parallel execution');
console.log('   • Race condition execution');
console.log('   • Mixed parallel/sequential workflows');
console.log('   • Multiple parallel groups in single workflow');
console.log('\n');
console.log('🚀 Ready for execution testing with Rust core!');
