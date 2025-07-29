# Performance Test Workflow

Comprehensive performance testing and benchmarking system for Cronflow

```typescript
const express = require('express');
const fs = require('fs');
const path = require('path');
const cronflowModule = require('cronflow');
const cronflow = cronflowModule.default || cronflowModule.cronflow;

const app = express();
app.use(express.json());

// Simple monitoring utilities
const monitor = {
  startTime: 0,
  stepTimes: [],
  memoryUsage: [],
  workflowId: '',
  lastStepTime: 0,

  start(workflowId) {
    this.workflowId = workflowId;
    this.startTime = performance.now();
    this.lastStepTime = this.startTime;
    this.stepTimes = [];
    this.memoryUsage = [];
    this.logMemory('workflow_start');
    console.log(
      `🚀 Performance monitoring started for workflow: ${workflowId}`
    );
    console.log(`🔍 DEBUG: Monitoring start time: ${this.startTime}`);
  },

  logStep(stepName) {
    const currentTime = performance.now();
    const stepDuration = currentTime - this.lastStepTime; // Actual step duration

    console.log(`🔍 DEBUG: ${stepName}`);
    console.log(`   Current time: ${currentTime}`);
    console.log(`   Last step time: ${this.lastStepTime}`);
    console.log(`   Step duration: ${stepDuration}ms`);
    console.log(`   Workflow start time: ${this.startTime}`);
    console.log(`   Total elapsed: ${currentTime - this.startTime}ms`);

    this.stepTimes.push({
      step: stepName,
      startTime: this.lastStepTime,
      endTime: currentTime,
      duration: stepDuration,
    });

    this.lastStepTime = currentTime; // Update for next step
    this.logMemory(stepName);

    // Log step performance immediately
    const memUsage = process.memoryUsage();
    console.log(
      `📊 Step: ${stepName} | Duration: ${stepDuration.toFixed(2)}ms | Memory: ${(memUsage.rss / 1024 / 1024).toFixed(2)}MB`
    );
  },

  logMemory(stepName) {
    const memUsage = process.memoryUsage();
    this.memoryUsage.push({
      step: stepName,
      timestamp: Date.now(),
      rss: memUsage.rss,
      heapUsed: memUsage.heapUsed,
      heapTotal: memUsage.heapTotal,
      external: memUsage.external,
    });
  },

  finish() {
    const totalTime = performance.now() - this.startTime;
    this.logMemory('workflow_end');

    const report = this.generateReport(totalTime);
    this.saveReport(report);
    this.logToConsole(report);

    return report;
  },

  generateReport(totalTime) {
    const memoryStart = this.memoryUsage[0];
    const memoryEnd = this.memoryUsage[this.memoryUsage.length - 1];
    const memoryDiff = memoryEnd.rss - memoryStart.rss;

    return {
      workflowId: this.workflowId,
      totalExecutionTime: totalTime,
      totalSteps: this.stepTimes.length,
      averageStepTime:
        this.stepTimes.reduce((sum, step) => sum + step.duration, 0) /
        this.stepTimes.length,
      fastestStep: this.stepTimes.reduce((min, step) =>
        step.duration < min.duration ? step : min
      ),
      slowestStep: this.stepTimes.reduce((max, step) =>
        step.duration > max.duration ? step : max
      ),
      memoryUsage: {
        initial: memoryStart.rss,
        final: memoryEnd.rss,
        diff: memoryDiff,
        diffMB: (memoryDiff / 1024 / 1024).toFixed(2),
        peak: Math.max(...this.memoryUsage.map(m => m.rss)),
        peakMB: (
          Math.max(...this.memoryUsage.map(m => m.rss)) /
          1024 /
          1024
        ).toFixed(2),
      },
      stepDetails: this.stepTimes,
      memoryDetails: this.memoryUsage,
      timestamp: new Date().toISOString(),
    };
  },

  saveReport(report) {
    const fileName = `performance_report_${Date.now()}.json`;
    const filePath = path.join(__dirname, 'performance_logs', fileName);

    // Create directory if it doesn't exist
    const dir = path.dirname(filePath);
    if (!fs.existsSync(dir)) {
      fs.mkdirSync(dir, { recursive: true });
    }

    fs.writeFileSync(filePath, JSON.stringify(report, null, 2));
    console.log(`📊 Performance report saved to: ${fileName}`);
  },

  logToConsole(report) {
    console.log('\n' + '='.repeat(80));
    console.log('🚀 CRONFLOW PERFORMANCE REPORT');
    console.log('='.repeat(80));
    console.log(`📋 Workflow ID: ${report.workflowId}`);
    console.log(
      `⏱️  Total Execution Time: ${report.totalExecutionTime.toFixed(3)}ms`
    );
    console.log(`📊 Total Steps: ${report.totalSteps}`);
    console.log(`⚡ Average Step Time: ${report.averageStepTime.toFixed(3)}ms`);
    console.log(
      `🏆 Fastest Step: ${report.fastestStep.step} (${report.fastestStep.duration.toFixed(3)}ms)`
    );
    console.log(
      `🐌 Slowest Step: ${report.slowestStep.step} (${report.slowestStep.duration.toFixed(3)}ms)`
    );
    console.log(
      `💾 Memory Usage: ${report.memoryUsage.diffMB}MB (Peak: ${report.memoryUsage.peakMB}MB)`
    );
    console.log(
      `📈 Memory per Step: ${(parseFloat(report.memoryUsage.diffMB) / report.totalSteps).toFixed(3)}MB`
    );
    console.log('='.repeat(80));

    // Log step details
    console.log('\n📋 STEP BREAKDOWN:');
    report.stepDetails.forEach((step, index) => {
      console.log(
        `${(index + 1).toString().padStart(2, '0')}. ${step.step.padEnd(25)} ${step.duration.toFixed(3)}ms`
      );
    });
    console.log('='.repeat(80) + '\n');
  },
};

// Complex calculation functions
function fibonacciCalculation(n = 35) {
  function fib(num) {
    if (num <= 1) return num;
    return fib(num - 1) + fib(num - 2);
  }
  return fib(n);
}

function matrixMultiplication(size = 50) {
  const matrixA = Array.from({ length: size }, () =>
    Array.from({ length: size }, () => Math.random() * 100)
  );
  const matrixB = Array.from({ length: size }, () =>
    Array.from({ length: size }, () => Math.random() * 100)
  );

  const result = Array.from({ length: size }, () => Array(size).fill(0));

  for (let i = 0; i < size; i++) {
    for (let j = 0; j < size; j++) {
      for (let k = 0; k < size; k++) {
        result[i][j] += matrixA[i][k] * matrixB[k][j];
      }
    }
  }

  return result.length;
}

function primeCalculation(limit = 10000) {
  const primes = [];
  const isPrime = num => {
    if (num < 2) return false;
    for (let i = 2; i <= Math.sqrt(num); i++) {
      if (num % i === 0) return false;
    }
    return true;
  };

  for (let i = 2; i < limit; i++) {
    if (isPrime(i)) primes.push(i);
  }

  return primes.length;
}

function sortingBenchmark(size = 10000) {
  const data = Array.from({ length: size }, () => Math.random() * 1000000);

  // Multiple sorting algorithms
  const quickSorted = [...data].sort((a, b) => a - b);
  const bubbleSort = arr => {
    const sorted = [...arr];
    for (let i = 0; i < sorted.length; i++) {
      for (let j = 0; j < sorted.length - i - 1; j++) {
        if (sorted[j] > sorted[j + 1]) {
          [sorted[j], sorted[j + 1]] = [sorted[j + 1], sorted[j]];
        }
      }
    }
    return sorted;
  };

  return {
    quickSortLength: quickSorted.length,
    dataSize: size,
  };
}

function dataProcessingBenchmark(records = 5000) {
  const data = Array.from({ length: records }, (_, i) => ({
    id: i,
    name: `User_${i}`,
    score: Math.random() * 100,
    category: ['A', 'B', 'C', 'D'][Math.floor(Math.random() * 4)],
    timestamp: Date.now() - Math.random() * 1000000,
  }));

  // Complex data processing
  const processed = data
    .filter(item => item.score > 30)
    .map(item => ({
      ...item,
      grade:
        item.score > 90
          ? 'A'
          : item.score > 80
            ? 'B'
            : item.score > 70
              ? 'C'
              : 'D',
      processed: true,
    }))
    .sort((a, b) => b.score - a.score)
    .reduce((acc, item) => {
      acc[item.category] = acc[item.category] || [];
      acc[item.category].push(item);
      return acc;
    }, {});

  return {
    originalCount: records,
    processedCount: Object.values(processed).flat().length,
    categories: Object.keys(processed).length,
  };
}

// Define the complex test workflow
const testWorkflow = cronflow.define({
  id: 'performance-test-workflow',
  name: 'CronFlow Performance Test Workflow',
  description:
    'A comprehensive workflow to test performance with 10 steps including parallel execution',
});

// Build the 10-step workflow with parallel execution
testWorkflow
  .step('step_1_initialization', async ctx => {
    console.log('🚀 Step 1: Workflow initialization');
    const stepStart = performance.now();

    const startData = {
      workflowId: ctx.workflowId || 'test-workflow',
      startTime: Date.now(),
      initialMemory: process.memoryUsage().rss,
    };

    // Some computational work
    let sum = 0;
    for (let i = 0; i < 100000; i++) {
      sum += Math.sqrt(i) * Math.sin(i);
    }

    const stepEnd = performance.now();
    const actualStepDuration = stepEnd - stepStart;
    console.log(
      `🔍 ACTUAL Step 1 duration: ${actualStepDuration.toFixed(2)}ms`
    );

    monitor.logStep('step_1_initialization');
    return { ...startData, computationResult: sum, step: 1 };
  })

  .step('step_2_data_generation', async ctx => {
    console.log('📊 Step 2: Large data generation');

    const largeDataset = Array.from({ length: 10000 }, (_, i) => ({
      id: i,
      value: Math.random() * 1000,
      category: `cat_${i % 10}`,
      timestamp: Date.now() + i,
    }));

    monitor.logStep('step_2_data_generation');
    return {
      dataSize: largeDataset.length,
      sampleData: largeDataset.slice(0, 5),
      step: 2,
      previousResult: ctx.last.computationResult,
    };
  })

  .step('step_3_fibonacci_calculation', async ctx => {
    console.log('🔢 Step 3: Fibonacci calculation (CPU intensive)');

    const fibResult = fibonacciCalculation(30); // Reduced for faster execution

    monitor.logStep('step_3_fibonacci_calculation');
    return {
      fibonacciResult: fibResult,
      step: 3,
      dataFromPrevious: ctx.last.dataSize,
    };
  })

  .step('step_4_matrix_operations', async ctx => {
    console.log('🧮 Step 4: Matrix multiplication');

    const matrixResult = matrixMultiplication(30); // Reduced size

    monitor.logStep('step_4_matrix_operations');
    return {
      matrixOperationResult: matrixResult,
      step: 4,
      fibFromPrevious: ctx.last.fibonacciResult,
    };
  })

  .step('step_5_prime_generation', async ctx => {
    console.log('🔢 Step 5: Prime number generation');

    const primeCount = primeCalculation(5000); // Reduced limit

    monitor.logStep('step_5_prime_generation');
    return {
      primeCount: primeCount,
      step: 5,
      matrixFromPrevious: ctx.last.matrixOperationResult,
    };
  })

  // Parallel execution step (3 parallel operations)
  .parallel([
    async ctx => {
      console.log('🔄 Parallel Step A: Sorting benchmark');

      const sortResult = sortingBenchmark(5000);

      // Simulate some async work
      await new Promise(resolve => setTimeout(resolve, Math.random() * 100));

      monitor.logStep('parallel_step_a');
      return {
        parallelA: true,
        sortResult: sortResult,
        processingTime: Date.now(),
      };
    },

    async ctx => {
      console.log('🔄 Parallel Step B: Data processing');

      const processResult = dataProcessingBenchmark(3000);

      // Simulate API call delay
      await new Promise(resolve => setTimeout(resolve, Math.random() * 150));

      monitor.logStep('parallel_step_b');
      return {
        parallelB: true,
        processResult: processResult,
        processingTime: Date.now(),
      };
    },

    async ctx => {
      console.log('🔄 Parallel Step C: Complex calculations');

      // Complex mathematical operations
      let result = 0;
      for (let i = 0; i < 50000; i++) {
        result += Math.pow(Math.sin(i), 2) + Math.cos(i) * Math.tan(i / 1000);
      }

      await new Promise(resolve => setTimeout(resolve, Math.random() * 75));

      monitor.logStep('parallel_step_c');
      return {
        parallelC: true,
        complexCalculationResult: result,
        processingTime: Date.now(),
      };
    },
  ])

  .step('step_7_data_aggregation', async ctx => {
    console.log('📈 Step 7: Data aggregation from parallel results');

    const parallelResults = ctx.last;
    const aggregatedData = {
      sortedDataSize: parallelResults[0].sortResult.dataSize,
      processedRecords: parallelResults[1].processResult.processedCount,
      calculationResult: parallelResults[2].complexCalculationResult,
      totalParallelTime:
        Math.max(...parallelResults.map(r => r.processingTime)) -
        Math.min(...parallelResults.map(r => r.processingTime)),
    };

    monitor.logStep('step_7_data_aggregation');
    return {
      aggregatedData: aggregatedData,
      step: 7,
      parallelStepsCompleted: parallelResults.length,
    };
  })

  .step('step_8_validation_check', async ctx => {
    console.log('✅ Step 8: Data validation and integrity check');

    const validation = {
      dataIntegrityCheck: ctx.last.aggregatedData.sortedDataSize > 0,
      processedRecordsValid: ctx.last.aggregatedData.processedRecords > 0,
      calculationValid: !isNaN(ctx.last.aggregatedData.calculationResult),
      allTestsPassed: true,
    };

    validation.allTestsPassed = Object.values(validation).every(
      v => v === true || typeof v === 'number'
    );

    monitor.logStep('step_8_validation_check');
    return {
      validation: validation,
      step: 8,
      previousStepData: ctx.last.aggregatedData,
    };
  })

  .step('step_9_final_computation', async ctx => {
    console.log('🎯 Step 9: Final computation and summary');

    // Final complex computation combining all previous results
    const finalResult = {
      totalStepsExecuted: 9,
      validationPassed: ctx.last.validation.allTestsPassed,
      workflowExecutionSummary: {
        sortedElements: ctx.last.previousStepData.sortedDataSize,
        processedRecords: ctx.last.previousStepData.processedRecords,
        calculationComplexity: Math.abs(
          ctx.last.previousStepData.calculationResult
        ).toFixed(2),
      },
      performanceMetrics: {
        currentMemory: process.memoryUsage().rss,
        currentTime: Date.now(),
      },
    };

    monitor.logStep('step_9_final_computation');
    return {
      finalResult: finalResult,
      step: 9,
      workflowNearCompletion: true,
    };
  })

  .step('step_10_cleanup_and_report', async ctx => {
    console.log('🧹 Step 10: Cleanup and final report generation');

    const workflowSummary = {
      workflowId: ctx.workflowId || 'performance-test',
      totalSteps: 10,
      parallelStepsExecuted: 3,
      finalValidation: ctx.last.finalResult.validationPassed,
      completionTime: Date.now(),
      memoryAtCompletion: process.memoryUsage(),
      status: 'COMPLETED_SUCCESSFULLY',
    };

    // Cleanup operations (simulate)
    global.gc && global.gc(); // Force garbage collection if available

    monitor.logStep('step_10_cleanup_and_report');

    // Generate final performance report
    const report = monitor.finish();
    console.log(
      '🎉 Workflow completed successfully with performance monitoring!'
    );

    return {
      workflowSummary: workflowSummary,
      step: 10,
      finalStep: true,
      message:
        'Workflow execution completed successfully with performance monitoring!',
    };
  });

// Express routes
app.get('/', (req, res) => {
  res.json({
    message: 'CronFlow Performance Test Server',
    endpoints: {
      '/trigger': 'POST - Trigger the performance test workflow',
      '/status': 'GET - Server status',
      '/logs': 'GET - List performance logs',
    },
  });
});

app.get('/status', (req, res) => {
  res.json({
    status: 'running',
    memory: process.memoryUsage(),
    uptime: process.uptime(),
    cronflowStatus: 'ready',
  });
});

app.get('/debug', (req, res) => {
  res.json({
    status: 'debug',
    memory: process.memoryUsage(),
    uptime: process.uptime(),
    cronflowStatus: 'ready',
    workflowRegistered: typeof testWorkflow !== 'undefined',
    workflowId: 'performance-test-workflow',
  });
});

app.get('/logs', (req, res) => {
  const logsDir = path.join(__dirname, 'performance_logs');
  if (!fs.existsSync(logsDir)) {
    return res.json({ logs: [], message: 'No logs directory found' });
  }

  const files = fs.readdirSync(logsDir).filter(file => file.endsWith('.json'));
  res.json({ logs: files, count: files.length });
});

// Alternative webhook-based workflow for testing
const webhookTestWorkflow = cronflow.define({
  id: 'webhook-performance-test',
  name: 'Webhook Performance Test Workflow',
  description: 'Alternative webhook-based test workflow',
  hooks: {
    onStart: ctx => {
      console.log('🚀 Webhook workflow started, initializing monitoring...');
      monitor.start('webhook-test');
    },
    onSuccess: (ctx, stepId) => {
      if (!stepId) {
        // Workflow completed
        const report = monitor.finish();
        console.log('✅ Webhook test completed with monitoring');
      }
    },
  },
});

webhookTestWorkflow
  .onWebhook('/test-webhook', {
    app: 'express',
    appInstance: app,
    method: 'POST',
  })
  .step('simple-test', async ctx => {
    console.log('🚀 Webhook workflow triggered successfully!');

    let result = 0;
    for (let i = 0; i < 10000; i++) {
      result += Math.sqrt(i);
    }

    monitor.logStep('simple-test');
    return {
      success: true,
      result: result,
      performance: {
        executionTime: Date.now(),
        memoryUsed: process.memoryUsage().rss,
      },
    };
  });

app.post('/webhook-test', (req, res) => {
  // This will be handled by CronFlow webhook
  res.json({ message: 'This endpoint is handled by CronFlow webhook' });
});

app.post('/simple-trigger', async (req, res) => {
  try {
    console.log('\n🧪 Testing simple HTTP call to webhook...');

    // Make HTTP call to our own webhook endpoint
    const response = await fetch(`http://localhost:${PORT}/test-webhook`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ test: true, timestamp: Date.now() }),
    });

    const result = await response.json();

    res.json({
      success: true,
      message: 'Webhook test completed',
      result: result,
    });
  } catch (error) {
    console.error('❌ Webhook test failed:', error);
    res.status(500).json({
      success: false,
      error: error.message,
    });
  }
});

app.post('/trigger', async (req, res) => {
  try {
    console.log('\n🚀 Starting CronFlow performance test workflow...');

    // Add timeout to prevent hanging
    const timeoutPromise = new Promise((_, reject) => {
      setTimeout(
        () => reject(new Error('Workflow trigger timeout after 30 seconds')),
        30000
      );
    });

    const workflowId = `test_${Date.now()}`;

    // Initialize monitoring here since workflow hooks aren't working
    monitor.start(workflowId);

    console.log('📋 Workflow ID:', workflowId);
    console.log('🔍 Attempting to trigger workflow...');

    // Add race condition with timeout
    const runId = await Promise.race([
      cronflow.trigger('performance-test-workflow', {
        workflowId: workflowId,
        testParams: {
          timestamp: Date.now(),
          requestId: req.headers['x-request-id'] || 'manual-trigger',
        },
      }),
      timeoutPromise,
    ]);

    console.log('✅ Workflow triggered successfully, runId:', runId);

    res.json({
      success: true,
      message: 'Performance test workflow triggered successfully',
      runId: runId,
      workflowId: workflowId,
      timestamp: new Date().toISOString(),
    });
  } catch (error) {
    console.error('❌ Error triggering workflow:', error);
    console.error('❌ Error stack:', error.stack);
    res.status(500).json({
      success: false,
      error: error.message,
      stack: error.stack,
      timestamp: new Date().toISOString(),
    });
  }
});

// Start server
const PORT = process.env.PORT || 3000;

app.listen(PORT, async () => {
  console.log(`🚀 CronFlow Performance Test Server running on port ${PORT}`);
  console.log(`📊 Performance monitoring enabled`);
  console.log(`🔗 Trigger workflow: POST http://localhost:${PORT}/trigger`);

  try {
    console.log('🔄 Starting CronFlow engine...');
    await cronflow.start();
    console.log('✅ CronFlow engine started successfully');

    // Verify workflow is registered
    console.log('🔍 Checking if workflow is registered...');
    // Add a small delay to ensure everything is ready
    await new Promise(resolve => setTimeout(resolve, 1000));

    console.log('✅ Server is ready to handle requests');
    console.log('💡 Try: curl -X POST http://localhost:' + PORT + '/trigger');

    // Auto-trigger the workflow after a short delay
    console.log('🚀 Auto-triggering workflow in 2 seconds...');
    setTimeout(async () => {
      try {
        console.log(
          '\n🎯 AUTO-TRIGGER: Starting CronFlow performance test workflow...'
        );

        const workflowId = `auto_test_${Date.now()}`;
        monitor.start(workflowId);

        console.log('📋 Auto Workflow ID:', workflowId);
        console.log('🔍 Auto-triggering workflow...');

        const runId = await cronflow.trigger('performance-test-workflow', {
          workflowId: workflowId,
          testParams: {
            timestamp: Date.now(),
            requestId: 'auto-trigger',
          },
        });

        console.log('✅ Auto workflow triggered successfully, runId:', runId);
        console.log(
          '🎉 Auto workflow completed! Check the performance report above.'
        );
      } catch (error) {
        console.error('❌ Auto workflow trigger failed:', error);
      }
    }, 2000);
  } catch (error) {
    console.error('❌ Failed to start CronFlow engine:', error);
    console.error('❌ Error details:', error.stack);
    process.exit(1);
  }
});
```
