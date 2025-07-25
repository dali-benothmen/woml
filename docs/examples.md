# Node-Cronflow Examples

Comprehensive examples showcasing the full power of node-cronflow for building reliable, scalable workflow automation.

## Table of Contents

1. [Basic Workflow](#basic-workflow)
2. [Service Integration](#service-integration)
3. [Conditional Logic](#conditional-logic)
4. [Framework Integration](#framework-integration)
5. [Advanced Control Flow](#advanced-control-flow)
6. [Human-in-the-Loop Approval](#human-in-the-loop-approval)
7. [Testing Workflows](#testing-workflows)
8. [Complete Real-World Examples](#complete-real-world-examples)

---

## Basic Workflow

A simple webhook-triggered workflow with basic steps and actions:

```typescript
import { cronflow } from 'cronflow';
import { z } from 'zod';

const basicWorkflow = cronflow.define({
  id: 'basic-webhook-workflow',
  name: 'Basic Webhook Workflow',
  description: 'Simple workflow triggered by webhook',
  tags: ['demo', 'webhook'],
  timeout: '5m',
  concurrency: 10,
  hooks: {
    onSuccess: (ctx) => {
      console.log('🎉 Workflow completed successfully!');
      console.log('Final output:', ctx.last);
    },
    onFailure: (ctx) => {
      console.error('💥 Workflow failed:', ctx.error);
    },
  },
});

basicWorkflow
  .onWebhook('/webhooks/basic', {
    method: 'POST',
    schema: z.object({
      message: z.string().min(1),
      userId: z.string().optional(),
    }),
    parseRawBody: false,
    headers: {
      required: {
        'content-type': 'application/json',
      },
    },
  })
  .step('validate-input', async (ctx) => {
    console.log('📝 Validating input:', ctx.payload);
    return { validated: true, message: ctx.payload.message };
  })
  .step('process-data', async (ctx) => {
    console.log('📝 Processing data:', ctx.last);
    return {
      processed: true,
      result: ctx.last.message.toUpperCase(),
      timestamp: new Date().toISOString(),
    };
  })
  .action('log-success', (ctx) => {
    console.log('✅ Processing completed successfully');
  });

// Self-executing function to start the workflow
(async () => {
  try {
    console.log('🚀 Starting basic workflow...');
    
    await cronflow.start({
      webhookServer: {
        host: '127.0.0.1',
        port: 3000,
        maxConnections: 1000,
      },
    });

    console.log('✅ Basic workflow started successfully');
    console.log('🌐 Webhook endpoint: http://127.0.0.1:3000/webhooks/basic');
    console.log('📋 Test with: curl -X POST http://127.0.0.1:3000/webhooks/basic \\');
    console.log('  -H "Content-Type: application/json" \\');
    console.log('  -d \'{"message": "Hello Cronflow!"}\'');
  } catch (error) {
    console.error('❌ Failed to start workflow:', error);
  }
})();
```

---

## Service Integration

Define and use external services with type safety:

```typescript
import { cronflow, defineService } from 'cronflow';
import { z } from 'zod';

// 1. Define service templates
const emailServiceTemplate = defineService({
  id: 'email-service',
  name: 'Email Service',
  description: 'Send emails via external API',
  version: '1.0.0',
  schema: {
    auth: z.object({
      apiKey: z.string(),
      fromEmail: z.string().email(),
    }),
  },
  setup: ({ auth }) => {
    return {
      actions: {
        sendEmail: async (params: {
          to: string;
          subject: string;
          body: string;
        }) => {
          // Simulate email sending
          console.log(`📧 Sending email to ${params.to}: ${params.subject}`);
          return { id: `email_${Date.now()}`, status: 'sent' };
        },
      },
    };
  },
});

const paymentServiceTemplate = defineService({
  id: 'payment-service',
  name: 'Payment Service',
  description: 'Process payments via Stripe',
  version: '1.0.0',
  schema: {
    auth: z.object({
      apiKey: z.string(),
    }),
  },
  setup: ({ auth }) => {
    return {
      actions: {
        processPayment: async (params: {
          amount: number;
          currency: string;
          customerId: string;
        }) => {
          console.log(`💳 Processing payment: $${params.amount} ${params.currency}`);
          return {
            id: `txn_${Date.now()}`,
            status: 'succeeded',
            amount: params.amount,
          };
        },
      },
    };
  },
});

// 2. Create configured service instances
const emailService = emailServiceTemplate.withConfig({
  auth: {
    apiKey: process.env.EMAIL_API_KEY!,
    fromEmail: 'noreply@example.com',
  },
});

const paymentService = paymentServiceTemplate.withConfig({
  auth: {
    apiKey: process.env.STRIPE_API_KEY!,
  },
});

// 3. Define workflow with services
const serviceWorkflow = cronflow.define({
  id: 'service-integration-example',
  name: 'Service Integration Example',
  description: 'Demonstrates service integration patterns',
  services: [emailService, paymentService],
  timeout: '10m',
  concurrency: 5,
  hooks: {
    onSuccess: (ctx) => {
      console.log('🎉 Service workflow completed successfully!');
    },
    onFailure: (ctx) => {
      console.error('💥 Service workflow failed:', ctx.error);
    },
  },
});

// 4. Use services in workflow
serviceWorkflow
  .onWebhook('/webhooks/service-demo', {
    schema: z.object({
      userId: z.string(),
      amount: z.number().positive(),
      email: z.string().email(),
    }),
  })
  .step('fetch-user-data', async (ctx) => {
    console.log('👤 Fetching user data for:', ctx.payload.userId);
    return {
      user: {
        id: ctx.payload.userId,
        email: ctx.payload.email,
        name: 'John Doe',
      },
    };
  })
  .step('process-payment', async (ctx) => {
    const payment = await ctx.services['payment-service'].processPayment({
      amount: ctx.payload.amount,
      currency: 'USD',
      customerId: ctx.last.user.id,
    });
    return { payment, processed: true };
  })
  .step('send-confirmation', async (ctx) => {
    const email = await ctx.services['email-service'].sendEmail({
      to: ctx.last.user.email,
      subject: 'Payment Confirmation',
      body: `Your payment of $${ctx.last.payment.amount} has been processed successfully.`,
    });
    return { email, sent: true };
  })
  .action('log-completion', (ctx) => {
    console.log('✅ Payment processed and confirmation sent');
  });

// Start the workflow
(async () => {
  try {
    console.log('🚀 Starting service integration workflow...');
    
    await cronflow.start({
      webhookServer: {
        host: '127.0.0.1',
        port: 3000,
  },
});

    console.log('✅ Service workflow started successfully');
    console.log('🌐 Webhook endpoint: http://127.0.0.1:3000/webhooks/service-demo');
  } catch (error) {
    console.error('❌ Failed to start workflow:', error);
  }
})();
```

---

## Conditional Logic

Advanced conditional workflows with if/else, parallel execution, and background actions:

```typescript
import { cronflow } from 'cronflow';
import { z } from 'zod';

const conditionalWorkflow = cronflow.define({
  id: 'conditional-workflow-example',
  name: 'Conditional Logic Example',
  description: 'Demonstrates if/else, parallel execution, and background actions',
  tags: ['conditional', 'parallel', 'background'],
  timeout: '15m',
  hooks: {
    onSuccess: (ctx, stepId) => {
      if (!stepId) {
        console.log('🎉 Conditional workflow completed successfully!');
        console.log('Final output:', ctx.last);
      } else {
        console.log(`✅ Step ${stepId} completed:`, ctx.step_result);
      }
    },
    onFailure: (ctx, stepId) => {
      if (!stepId) {
        console.log('💥 Conditional workflow failed:', ctx.error);
      } else {
        console.log(`❌ Step ${stepId} failed:`, ctx.step_error);
      }
    },
  },
});

conditionalWorkflow
  .onWebhook('/webhooks/conditional', {
    schema: z.object({
      amount: z.number().positive(),
      description: z.string().optional(),
      priority: z.enum(['low', 'medium', 'high']).optional(),
    }),
  })
  .step('validate-amount', async (ctx) => {
    console.log('🔍 Validating amount:', ctx.payload.amount);
    return { amount: ctx.payload.amount, validated: true };
  })
  .if('is-high-value', (ctx) => {
    console.log('🔍 Evaluating high-value condition');
    return ctx.last.amount > 500;
  })
  .step('process-high-value', async (ctx) => {
    console.log('💎 Processing high-value transaction');
    return { type: 'high-value', processed: true, amount: ctx.last.amount };
  })
  .parallel([
    async (ctx) => {
      console.log('🔄 Parallel step 1: Validate data');
      await new Promise(resolve => setTimeout(resolve, 200));
      return {
        validation: 'success',
        amount: ctx.last.amount,
        timestamp: new Date().toISOString(),
      };
    },
    async (ctx) => {
      console.log('🔄 Parallel step 2: Log transaction');
      await new Promise(resolve => setTimeout(resolve, 150));
    return {
        logged: true,
        transactionId: `txn_${Date.now()}`,
        amount: ctx.last.amount,
      };
    },
  ])
  .action('background-notification', async (ctx) => {
    console.log('🔄 Background action: Sending notification');
    await new Promise(resolve => setTimeout(resolve, 1000));
    console.log('✅ Background notification sent');
    return { notification: 'sent', timestamp: new Date().toISOString() };
  })
  .endIf()
  .elseIf('is-medium-value', (ctx) => {
    console.log('🔍 Evaluating medium-value condition');
    return ctx.last.amount > 100;
  })
  .step('process-medium-value', async (ctx) => {
    console.log('📊 Processing medium-value transaction');
    return { type: 'medium-value', processed: true, amount: ctx.last.amount };
  })
  .endIf()
  .else()
  .step('process-low-value', async (ctx) => {
    console.log('📝 Processing low-value transaction');
    return { type: 'low-value', processed: true, amount: ctx.last.amount };
  })
  .endIf()
  .step('final-summary', async (ctx) => {
    console.log('📋 Creating final summary');
    return {
      final: true,
      summary: ctx.last,
      completedAt: new Date().toISOString(),
    };
  });

// Start the conditional workflow
(async () => {
  try {
    console.log('🚀 Starting conditional workflow...');
    
    await cronflow.start({
      webhookServer: {
        host: '127.0.0.1',
        port: 3000,
  },
});

    console.log('✅ Conditional workflow started successfully');
    console.log('🌐 Webhook endpoint: http://127.0.0.1:3000/webhooks/conditional');
    console.log('📋 Test examples:');
    console.log('  High value: curl -X POST http://127.0.0.1:3000/webhooks/conditional \\');
    console.log('    -H "Content-Type: application/json" \\');
    console.log('    -d \'{"amount": 750, "description": "High value order"}\'');
    console.log('  Medium value: curl -X POST http://127.0.0.1:3000/webhooks/conditional \\');
    console.log('    -H "Content-Type: application/json" \\');
    console.log('    -d \'{"amount": 250, "description": "Medium value order"}\'');
    console.log('  Low value: curl -X POST http://127.0.0.1:3000/webhooks/conditional \\');
    console.log('    -H "Content-Type: application/json" \\');
    console.log('    -d \'{"amount": 50, "description": "Low value order"}\'');
  } catch (error) {
    console.error('❌ Failed to start workflow:', error);
  }
})();
```

---

## Framework Integration

Integrate with Express.js, Fastify, and custom frameworks:

```typescript
import { cronflow } from 'cronflow';
import express from 'express';
import { z } from 'zod';

// Create Express app
const app = express();
const PORT = 3000;

// Set up middleware
app.use(express.json());
app.use(express.urlencoded({ extended: true }));

// Add request logging
app.use((req, res, next) => {
  console.log(`🌐 ${req.method} ${req.path} - ${new Date().toISOString()}`);
  next();
});

// Define workflow for framework integration
const frameworkWorkflow = cronflow.define({
  id: 'framework-integration-example',
  name: 'Framework Integration Example',
  description: 'Demonstrates integration with Express.js and custom endpoints',
  timeout: '10m',
  hooks: {
    onSuccess: (ctx) => {
      console.log('🎉 Framework workflow completed successfully!');
    },
    onFailure: (ctx) => {
      console.error('💥 Framework workflow failed:', ctx.error);
    },
  },
});

// Express.js integration
frameworkWorkflow
  .onWebhook('/api/webhooks/framework-test', {
    app: 'express',
    appInstance: app,
    method: 'POST',
    schema: z.object({
      message: z.string().min(1, 'Message cannot be empty'),
      userId: z.string().optional(),
      timestamp: z.string().optional(),
    }),
    headers: {
      required: { 'content-type': 'application/json' },
    },
  })
  .step('validate-input', async (ctx) => {
    console.log('📝 Validating input:', ctx.payload);
    return { validated: true, message: ctx.payload.message };
  })
  .step('process-data', async (ctx) => {
    console.log('📝 Processing data:', ctx.last);
    return {
      processed: true,
      result: ctx.last.message.toUpperCase(),
      timestamp: new Date().toISOString(),
    };
  })
  .step('finalize', async (ctx) => {
    console.log('📝 Finalizing:', ctx.last);
    return {
      final: true,
      summary: ctx.last,
      completedAt: new Date().toISOString(),
    };
  });

// Manual trigger endpoint
app.post('/api/trigger-workflow', async (req, res) => {
  try {
    console.log('🚀 Manual trigger endpoint called');
    console.log('Payload:', req.body);

    const runId = await cronflow.trigger('framework-integration-example', req.body);

    console.log('✅ Workflow triggered successfully');
    console.log('Run ID:', runId);

    res.json({
      success: true,
      runId,
      message: 'Workflow triggered successfully',
      timestamp: new Date().toISOString(),
    });
  } catch (error: any) {
    console.error('❌ Manual trigger failed:', error);
    res.status(500).json({
      success: false,
      error: error.message,
      timestamp: new Date().toISOString(),
    });
  }
});

// Workflow status endpoint
app.get('/api/workflows/:runId', async (req, res) => {
  try {
    const { runId } = req.params;
    console.log('📊 Checking workflow status for run:', runId);

    const status = await cronflow.inspect(runId);

    res.json({
      success: true,
      runId,
      status,
      timestamp: new Date().toISOString(),
    });
  } catch (error: any) {
    console.error('❌ Status check failed:', error);
    res.status(500).json({
      success: false,
      error: error.message,
      timestamp: new Date().toISOString(),
    });
  }
});

// Health check endpoint
app.get('/health', (req, res) => {
  res.json({
    status: 'healthy',
    timestamp: new Date().toISOString(),
    service: 'cronflow-framework-integration',
  });
});

// Start Express server
app.listen(PORT, async () => {
  console.log(`\n🚀 Express server started on port ${PORT}`);
  console.log(`🌐 Server URL: http://localhost:${PORT}`);
  console.log(`🔗 Health check: http://localhost:${PORT}/health`);
  console.log(`🚀 Manual trigger: POST http://localhost:${PORT}/api/trigger-workflow`);
  console.log(`🔗 Webhook endpoint: POST http://localhost:${PORT}/api/webhooks/framework-test`);
  console.log(`📊 Status check: GET http://localhost:${PORT}/api/workflows/:runId`);

  // Register workflow with Rust core
  try {
    console.log('\n🔧 Registering workflow with Rust core...');
    await cronflow.start(); // Register workflows with Rust core (no webhook server)
    console.log('✅ Workflow registered successfully');
  } catch (error) {
    console.error('❌ Failed to register workflow:', error);
  }

  console.log('\n📝 Test Instructions:');
  console.log('1. Test manual trigger:');
  console.log('   curl -X POST http://localhost:3000/api/trigger-workflow \\');
  console.log('     -H "Content-Type: application/json" \\');
  console.log('     -d \'{"message": "Hello from manual trigger!"}\'');
  console.log('');
  console.log('2. Test webhook trigger:');
  console.log('   curl -X POST http://localhost:3000/api/webhooks/framework-test \\');
  console.log('     -H "Content-Type: application/json" \\');
  console.log('     -d \'{"message": "Hello from webhook!"}\'');
  console.log('');
  console.log('3. Check workflow status:');
  console.log('   curl http://localhost:3000/api/workflows/{runId}');
  console.log('');
  console.log('✅ Framework integration test ready!');
});
```

---

## Advanced Control Flow

Demonstrating advanced features like race conditions, loops, and human-in-the-loop:

```typescript
import { cronflow } from 'cronflow';
import { z } from 'zod';

const advancedWorkflow = cronflow.define({
  id: 'advanced-control-flow',
  name: 'Advanced Control Flow Example',
  description: 'Demonstrates race conditions, loops, and human approval',
  timeout: '30m',
  hooks: {
    onSuccess: (ctx) => {
      console.log('🎉 Advanced workflow completed successfully!');
    },
    onFailure: (ctx) => {
      console.error('💥 Advanced workflow failed:', ctx.error);
    },
  },
});

advancedWorkflow
  .onWebhook('/webhooks/advanced', {
    schema: z.object({
      userId: z.string(),
      amount: z.number().positive(),
      requiresApproval: z.boolean().optional(),
    }),
  })
  .step('initialize', async (ctx) => {
    console.log('🚀 Initializing advanced workflow');
    return {
      userId: ctx.payload.userId,
      amount: ctx.payload.amount,
      requiresApproval: ctx.payload.requiresApproval || false,
      startedAt: new Date().toISOString(),
    };
  })
  .race([
    async (ctx) => {
      console.log('🏃‍♂️ Racing: Primary API call');
      await new Promise(resolve => setTimeout(resolve, 1000));
      return { source: 'primary', data: 'Primary API response' };
    },
    async (ctx) => {
      console.log('🏃‍♂️ Racing: Backup API call');
      await new Promise(resolve => setTimeout(resolve, 500));
      return { source: 'backup', data: 'Backup API response' };
    },
  ])
  .step('process-race-result', async (ctx) => {
    console.log('📊 Processing race result:', ctx.last);
    return {
      processed: true,
      source: ctx.last.source,
      data: ctx.last.data,
    };
  })
  .if('requires-approval', (ctx) => {
    console.log('🔍 Checking if approval is required');
    return ctx.steps.initialize.output.requiresApproval;
  })
  .humanInTheLoop({
    timeout: '24h', // Wait up to 24 hours for approval
    description: 'Approve high-value transaction',
    onPause: (token) => {
      console.log('⏸️ Workflow paused for human approval');
      console.log('Approval token:', token);
      // In real implementation, send email/Slack notification
    },
  })
  .step('process-approval', async (ctx) => {
    console.log('✅ Processing human approval result');
    
    if (ctx.last.timedOut) {
      console.log('⏰ Approval timed out - handling timeout scenario');
      return {
        approved: false,
        reason: 'Timeout - no approval received within 24 hours',
        handled: 'timeout',
        timestamp: new Date().toISOString(),
      };
    }
    
    return {
      approved: ctx.last.approved,
      approvedBy: ctx.last.approvedBy,
      reason: ctx.last.reason,
      timestamp: new Date().toISOString(),
    };
  })
  .endIf()
  .while('retry-loop', (ctx) => {
    const attempts = ctx.state.get('attempts', 0);
    console.log(`🔄 Retry loop attempt ${attempts + 1}`);
    return attempts < 3;
  }, (ctx) => {
    const attempts = ctx.state.get('attempts', 0);
    ctx.state.set('attempts', attempts + 1);
    console.log(`🔄 Processing attempt ${attempts + 1}`);
  })
  .step('final-processing', async (ctx) => {
    console.log('🎯 Final processing step');
    return {
      completed: true,
      finalResult: 'Success',
      completedAt: new Date().toISOString(),
    };
  });

// Start the advanced workflow
(async () => {
  try {
    console.log('🚀 Starting advanced control flow workflow...');
    
    await cronflow.start({
      webhookServer: {
        host: '127.0.0.1',
        port: 3000,
      },
    });

    console.log('✅ Advanced workflow started successfully');
    console.log('🌐 Webhook endpoint: http://127.0.0.1:3000/webhooks/advanced');
    console.log('📋 Test examples:');
    console.log('  With approval: curl -X POST http://127.0.0.1:3000/webhooks/advanced \\');
    console.log('    -H "Content-Type: application/json" \\');
    console.log('    -d \'{"userId": "user123", "amount": 1000, "requiresApproval": true}\'');
    console.log('  Without approval: curl -X POST http://127.0.0.1:3000/webhooks/advanced \\');
    console.log('    -H "Content-Type: application/json" \\');
    console.log('    -d \'{"userId": "user456", "amount": 100, "requiresApproval": false}\'');
  } catch (error) {
    console.error('❌ Failed to start workflow:', error);
  }
})();
```

---

## Human-in-the-Loop Approval

Demonstrating human approval workflows with timeout and indefinite pause scenarios:

```typescript
import { cronflow } from 'cronflow';
import { z } from 'zod';

const humanApprovalWorkflow = cronflow.define({
  id: 'human-approval-example',
  name: 'Human-in-the-Loop Approval Example',
  description: 'Demonstrates human approval workflows with timeout and resume functionality',
  timeout: '30m',
  hooks: {
    onSuccess: (ctx) => {
      console.log('🎉 Human approval workflow completed successfully!');
    },
    onFailure: (ctx) => {
      console.error('💥 Human approval workflow failed:', ctx.error);
    },
  },
});

humanApprovalWorkflow
  .onWebhook('/webhooks/human-approval', {
    schema: z.object({
      transactionId: z.string(),
      amount: z.number().positive(),
      description: z.string().optional(),
      requiresApproval: z.boolean().optional(),
    }),
  })
  .step('validate-transaction', async (ctx) => {
    console.log('🔍 Validating transaction:', ctx.payload.transactionId);
    return {
      transactionId: ctx.payload.transactionId,
      amount: ctx.payload.amount,
      description: ctx.payload.description,
      requiresApproval: ctx.payload.requiresApproval || ctx.payload.amount > 1000,
      validated: true,
    };
  })
  .if('needs-approval', (ctx) => {
    console.log('🔍 Checking if approval is required');
    return ctx.last.requiresApproval;
  })
  .humanInTheLoop({
    timeout: '24h', // Wait up to 24 hours for approval
    description: 'Approve high-value transaction',
    onPause: (token) => {
      console.log('🛑 Human approval required');
      console.log('🔑 Approval token:', token);
      console.log('📧 Send this token to approver for manual review');
      console.log('🔄 Use cronflow.resume(token, payload) to resume');
      console.log('📊 Transaction details:');
      console.log(`   - ID: ${ctx.steps['validate-transaction'].output.transactionId}`);
      console.log(`   - Amount: $${ctx.steps['validate-transaction'].output.amount}`);
      console.log(`   - Description: ${ctx.steps['validate-transaction'].output.description}`);
    },
  })
  .step('process-approval', async (ctx) => {
    console.log('✅ Processing human approval result');
    
    if (ctx.last.timedOut) {
      console.log('⏰ Approval timed out - handling timeout scenario');
      return {
        approved: false,
        reason: 'Timeout - no approval received within 24 hours',
        handled: 'timeout',
        timestamp: new Date().toISOString(),
      };
    }
    
    console.log('✅ Manual approval received:', ctx.last);
    return {
      approved: ctx.last.approved,
      approvedBy: ctx.last.approvedBy,
      reason: ctx.last.reason,
      timestamp: new Date().toISOString(),
    };
  })
  .endIf()
  .step('finalize-transaction', async (ctx) => {
    console.log('🎯 Finalizing transaction');
    return {
      finalized: true,
      transactionId: ctx.steps['validate-transaction'].output.transactionId,
      approvalStatus: ctx.last.approved ? 'approved' : 'rejected',
      completedAt: new Date().toISOString(),
    };
  });

// Start the human approval workflow
(async () => {
  try {
    console.log('🚀 Starting human approval workflow...');
    
    await cronflow.start({
      webhookServer: {
        host: '127.0.0.1',
        port: 3000,
      },
    });

    console.log('✅ Human approval workflow started successfully');
    console.log('🌐 Webhook endpoint: http://127.0.0.1:3000/webhooks/human-approval');
    console.log('📋 Test examples:');
    console.log('  With approval: curl -X POST http://127.0.0.1:3000/webhooks/human-approval \\');
    console.log('    -H "Content-Type: application/json" \\');
    console.log('    -d \'{"transactionId": "TXN-123", "amount": 5000, "description": "High-value purchase", "requiresApproval": true}\'');
    console.log('');
    console.log('  Without approval: curl -X POST http://127.0.0.1:3000/webhooks/human-approval \\');
    console.log('    -H "Content-Type: application/json" \\');
    console.log('    -d \'{"transactionId": "TXN-456", "amount": 500, "description": "Low-value purchase", "requiresApproval": false}\'');
    console.log('');
    console.log('🔄 Resume paused workflow:');
    console.log('   await cronflow.resume("TOKEN", {approved: true, reason: "Approved", approvedBy: "admin"});');
    console.log('');
    console.log('📊 List paused workflows:');
    console.log('   const paused = cronflow.listPausedWorkflows();');
  } catch (error) {
    console.error('❌ Failed to start workflow:', error);
  }
})();
```

### Indefinite Human Approval (No Timeout)

Example showing a workflow that pauses indefinitely until manually resumed:

```typescript
import { cronflow } from 'cronflow';
import { z } from 'zod';

const indefiniteApprovalWorkflow = cronflow.define({
  id: 'indefinite-approval-example',
  name: 'Indefinite Human Approval Example',
  description: 'Workflow that pauses indefinitely until manually resumed',
  timeout: '30m',
  hooks: {
    onSuccess: (ctx) => {
      console.log('🎉 Indefinite approval workflow completed successfully!');
    },
    onFailure: (ctx) => {
      console.error('💥 Indefinite approval workflow failed:', ctx.error);
    },
  },
});

indefiniteApprovalWorkflow
  .onWebhook('/webhooks/indefinite-approval', {
    schema: z.object({
      transactionId: z.string(),
      amount: z.number().positive(),
      requiresManualApproval: z.boolean().optional(),
    }),
  })
  .step('validate-transaction', async (ctx) => {
    console.log('🔍 Validating transaction:', ctx.payload.transactionId);
    return {
      transactionId: ctx.payload.transactionId,
      amount: ctx.payload.amount,
      validated: true,
    };
  })
  .if('needs-manual-approval', (ctx) => {
    return ctx.payload.requiresManualApproval || ctx.last.amount > 10000;
  })
  .humanInTheLoop({
    // No timeout - will pause indefinitely until manually resumed
    description: 'Manual approval required for high-value transaction',
    onPause: (token) => {
      console.log('🛑 Manual approval required - no timeout set');
      console.log('🔑 Approval token:', token);
      console.log('📧 Send this token to approver for manual review');
      console.log('🔄 Use cronflow.resume(token, payload) to resume');
      console.log('⚠️  This workflow will pause indefinitely until manually resumed');
    },
  })
  .step('process-manual-approval', async (ctx) => {
    console.log('✅ Manual approval received');
    console.log('   Approval details:', ctx.last);
    return {
      approved: ctx.last.approved,
      approvedBy: ctx.last.approvedBy,
      reason: ctx.last.reason,
      processed: true,
    };
  })
  .endIf()
  .step('finalize-transaction', async (ctx) => {
    console.log('🎯 Finalizing transaction');
    return {
      finalized: true,
      transactionId: ctx.steps['validate-transaction'].output.transactionId,
      completedAt: new Date().toISOString(),
    };
  });

// Start the indefinite approval workflow
(async () => {
  try {
    console.log('🚀 Starting indefinite approval workflow...');
    
    await cronflow.start({
      webhookServer: {
        host: '127.0.0.1',
        port: 3000,
      },
    });

    console.log('✅ Indefinite approval workflow started successfully');
    console.log('🌐 Webhook endpoint: http://127.0.0.1:3000/webhooks/indefinite-approval');
    console.log('📋 Test with: curl -X POST http://127.0.0.1:3000/webhooks/indefinite-approval \\');
    console.log('  -H "Content-Type: application/json" \\');
    console.log('  -d \'{"transactionId": "TXN-123", "amount": 15000, "requiresManualApproval": true}\'');
    console.log('');
    console.log('⚠️  Note: This workflow will pause indefinitely until manually resumed');
    console.log('   Use cronflow.resume(token, {approved: true, reason: "Approved"}) to resume');
  } catch (error) {
    console.error('❌ Failed to start workflow:', error);
  }
})();
```

### Resume and Management Functions

Examples of how to manage paused workflows:

```typescript
import { cronflow } from 'cronflow';

// Resume a paused workflow with approval
async function approveWorkflow(token: string) {
  try {
    await cronflow.resume(token, {
      approved: true,
      reason: 'Transaction looks good',
      approvedBy: 'admin@company.com',
    });
    console.log('✅ Workflow approved successfully');
  } catch (error) {
    console.error('❌ Failed to approve workflow:', error);
  }
}

// Resume a paused workflow with rejection
async function rejectWorkflow(token: string) {
  try {
    await cronflow.resume(token, {
      approved: false,
      reason: 'Amount too high for approval level',
      approvedBy: 'manager@company.com',
    });
    console.log('❌ Workflow rejected successfully');
  } catch (error) {
    console.error('❌ Failed to reject workflow:', error);
  }
}

// List all paused workflows
function listPausedWorkflows() {
  const paused = cronflow.listPausedWorkflows();
  console.log('📊 Paused workflows:', paused.length);
  
  paused.forEach(workflow => {
    console.log(`   - Token: ${workflow.token}`);
    console.log(`   - Description: ${workflow.description}`);
    console.log(`   - Created: ${new Date(workflow.createdAt).toISOString()}`);
    console.log(`   - Expires: ${workflow.expiresAt ? new Date(workflow.expiresAt).toISOString() : 'Never'}`);
    console.log(`   - Status: ${workflow.status}`);
    console.log('');
  });
  
  return paused;
}

// Get details of a specific paused workflow
function getPausedWorkflowDetails(token: string) {
  const workflow = cronflow.getPausedWorkflow(token);
  
  if (!workflow) {
    console.log('❌ Paused workflow not found');
    return null;
  }
  
  console.log('📊 Paused workflow details:');
  console.log(`   - Token: ${workflow.token}`);
  console.log(`   - Workflow ID: ${workflow.workflowId}`);
  console.log(`   - Run ID: ${workflow.runId}`);
  console.log(`   - Description: ${workflow.description}`);
  console.log(`   - Created: ${new Date(workflow.createdAt).toISOString()}`);
  console.log(`   - Expires: ${workflow.expiresAt ? new Date(workflow.expiresAt).toISOString() : 'Never'}`);
  console.log(`   - Status: ${workflow.status}`);
  console.log(`   - Metadata:`, workflow.metadata);
  
  return workflow;
}

// Example usage
(async () => {
  console.log('🔄 Human approval management examples:');
  
  // List all paused workflows
  listPausedWorkflows();
  
  // Get details of a specific workflow (replace with actual token)
  const token = 'human_approval_1234567890_abc123';
  getPausedWorkflowDetails(token);
  
  // Approve a workflow (replace with actual token)
  // await approveWorkflow(token);
  
  // Reject a workflow (replace with actual token)
  // await rejectWorkflow(token);
})();
```

---

## Testing Workflows

Comprehensive testing examples with mocking and expectations:

```typescript
import { cronflow } from 'cronflow';
import { z } from 'zod';

// Define a workflow for testing
const testableWorkflow = cronflow.define({
  id: 'testable-workflow',
  name: 'Testable Workflow Example',
  description: 'Workflow designed for comprehensive testing',
  timeout: '5m',
});

testableWorkflow
  .onWebhook('/webhooks/testable', {
    schema: z.object({
      userId: z.string(),
      amount: z.number().positive(),
    }),
  })
  .step('validate-input', async (ctx) => {
    if (!ctx.payload.userId) {
      throw new Error('UserId is required');
    }
    return { validated: true, userId: ctx.payload.userId };
  })
  .step('fetch-user', async (ctx) => {
    // Simulate API call
    const user = await fetchUserFromAPI(ctx.last.userId);
    return { user, fetched: true };
  })
  .step('process-payment', async (ctx) => {
    const payment = await processPayment({
      userId: ctx.last.user.id,
      amount: ctx.payload.amount,
    });
    return { payment, processed: true };
  })
  .action('send-notification', async (ctx) => {
    await sendNotification(ctx.last.user.email, 'Payment processed');
    return { notification: 'sent' };
  });

// Mock functions for testing
async function fetchUserFromAPI(userId: string) {
  return { id: userId, email: `${userId}@example.com`, name: 'Test User' };
}

async function processPayment(params: { userId: string; amount: number }) {
  return { id: `txn_${Date.now()}`, status: 'succeeded', amount: params.amount };
}

async function sendNotification(email: string, message: string) {
  console.log(`📧 Sending notification to ${email}: ${message}`);
}

// Testing examples
async function runTests() {
  console.log('🧪 Running workflow tests...');

  // Test 1: Successful workflow execution
  console.log('\n📋 Test 1: Successful workflow execution');
  const testRun1 = await testableWorkflow
      .test()
      .trigger({
      userId: 'user123',
      amount: 100,
    })
    .expectStep('validate-input')
      .toSucceed()
    .expectStep('fetch-user')
      .toSucceed()
    .expectStep('process-payment')
      .toSucceed()
      .run();

  console.log('✅ Test 1 passed:', testRun1.status);

  // Test 2: Failed validation
  console.log('\n📋 Test 2: Failed validation');
  const testRun2 = await testableWorkflow
      .test()
      .trigger({
      userId: '', // Invalid userId
      amount: 100,
    })
    .expectStep('validate-input')
    .toFailWith('UserId is required')
    .run();

  console.log('✅ Test 2 passed:', testRun2.status);

  // Test 3: Mocked step
  console.log('\n📋 Test 3: Mocked step');
  const testRun3 = await testableWorkflow
    .test()
    .trigger({
      userId: 'user456',
      amount: 200,
    })
    .mockStep('fetch-user', async (ctx) => {
      return { user: { id: 'mocked-user', email: 'mocked@example.com' }, fetched: true };
    })
    .expectStep('validate-input')
      .toSucceed()
    .expectStep('fetch-user')
    .toSucceed()
      .run();

  console.log('✅ Test 3 passed:', testRun3.status);

  // Test 4: Complex scenario with multiple mocks
  console.log('\n📋 Test 4: Complex scenario');
  const testRun4 = await testableWorkflow
    .test()
    .trigger({
      userId: 'user789',
      amount: 500,
    })
    .mockStep('fetch-user', async (ctx) => {
      return { user: { id: 'vip-user', email: 'vip@example.com' }, fetched: true };
    })
    .mockStep('process-payment', async (ctx) => {
      return { id: 'vip_txn_123', status: 'succeeded', amount: 500, vip: true };
    })
    .expectStep('validate-input')
    .toSucceed()
    .expectStep('fetch-user')
    .toSucceed()
    .expectStep('process-payment')
    .toSucceed()
    .run();

  console.log('✅ Test 4 passed:', testRun4.status);

  console.log('\n🎉 All tests completed successfully!');
}

// Start the testable workflow and run tests
(async () => {
  try {
    console.log('🚀 Starting testable workflow...');
    
    await cronflow.start({
      webhookServer: {
        host: '127.0.0.1',
        port: 3000,
      },
    });

    console.log('✅ Testable workflow started successfully');
    console.log('🌐 Webhook endpoint: http://127.0.0.1:3000/webhooks/testable');

    // Run tests
    await runTests();

  } catch (error) {
    console.error('❌ Failed to start workflow or run tests:', error);
  }
})();
```

---

## Complete Real-World Examples

### E-commerce Order Processing

A complete example showing order processing with multiple services:

```typescript
import { cronflow, defineService } from 'cronflow';
import { z } from 'zod';

// Define services
const stripeServiceTemplate = defineService({
  id: 'stripe',
  name: 'Stripe',
  description: 'Payment processing service',
  version: '1.0.0',
  schema: {
    auth: z.object({
      apiKey: z.string(),
      webhookSecret: z.string(),
    }),
  },
  setup: ({ auth }) => {
    return {
      actions: {
        validateWebhook: (payload: Buffer, signature: string) => {
          // Simulate webhook validation
          console.log('🔐 Validating Stripe webhook signature');
          return { valid: true, event: { type: 'checkout.session.completed', data: { object: payload } } };
        },
        processRefund: async (params: { paymentId: string; amount: number }) => {
          console.log(`💳 Processing refund for ${params.paymentId}: $${params.amount}`);
          return { id: `ref_${Date.now()}`, status: 'succeeded' };
        },
      },
    };
  },
});

const emailServiceTemplate = defineService({
  id: 'email',
  name: 'Email Service',
  description: 'Email delivery service',
  version: '1.0.0',
  schema: {
    auth: z.object({
      apiKey: z.string(),
    }),
  },
  setup: ({ auth }) => {
    return {
      actions: {
        sendOrderConfirmation: async (params: { to: string; orderId: string; amount: number }) => {
          console.log(`📧 Sending order confirmation to ${params.to}`);
          return { id: `email_${Date.now()}`, status: 'sent' };
        },
        sendRefundNotification: async (params: { to: string; refundId: string; amount: number }) => {
          console.log(`📧 Sending refund notification to ${params.to}`);
          return { id: `email_${Date.now()}`, status: 'sent' };
        },
      },
    };
  },
});

// Configure services
const stripeService = stripeServiceTemplate.withConfig({
  auth: {
    apiKey: process.env.STRIPE_API_KEY!,
    webhookSecret: process.env.STRIPE_WEBHOOK_SECRET!,
  },
});

const emailService = emailServiceTemplate.withConfig({
  auth: {
    apiKey: process.env.EMAIL_API_KEY!,
  },
});

// Define the order processing workflow
const orderProcessingWorkflow = cronflow.define({
  id: 'ecommerce-order-processing',
  name: 'E-commerce Order Processing',
  description: 'Complete order processing workflow with refund handling',
  services: [stripeService, emailService],
  timeout: '15m',
  concurrency: 10,
  hooks: {
    onSuccess: (ctx) => {
      console.log('🎉 Order processing completed successfully!');
    },
    onFailure: (ctx) => {
      console.error('💥 Order processing failed:', ctx.error);
    },
  },
});

orderProcessingWorkflow
  .onWebhook('/webhooks/stripe', {
    parseRawBody: true,
    schema: z.object({
      type: z.string(),
      data: z.object({
        object: z.object({
          id: z.string(),
          amount_total: z.number(),
          metadata: z.object({
            orderId: z.string(),
            customerEmail: z.string(),
          }),
        }),
      }),
    }),
  })
  .step('validate-webhook', async (ctx) => {
    const signature = ctx.trigger.headers['stripe-signature'];
    if (!signature) {
      throw new Error('Missing Stripe signature');
    }

    const event = ctx.services.stripe.validateWebhook(ctx.trigger.rawBody!, signature);
    
    if (event.event.type !== 'checkout.session.completed') {
      return ctx.cancel({ reason: `Ignoring event type: ${event.event.type}` });
    }

    return event.event.data.object;
  })
  .step('process-order', async (ctx) => {
    const session = ctx.last;
    console.log(`📦 Processing order ${session.metadata.orderId}`);
    
    return {
      orderId: session.metadata.orderId,
      customerEmail: session.metadata.customerEmail,
      amount: session.amount_total / 100, // Convert from cents
      processed: true,
    };
  })
  .if('is-high-value', (ctx) => {
    return ctx.last.amount > 500;
  })
  .step('vip-processing', async (ctx) => {
    console.log('💎 Processing VIP order');
    return { vip: true, priority: 'high' };
  })
  .endIf()
  .parallel([
    async (ctx) => {
      const email = await ctx.services.email.sendOrderConfirmation({
        to: ctx.last.customerEmail,
        orderId: ctx.last.orderId,
        amount: ctx.last.amount,
      });
      return { email, sent: true };
    },
    async (ctx) => {
      // Simulate inventory update
      console.log(`📦 Updating inventory for order ${ctx.last.orderId}`);
      return { inventory: 'updated' };
    },
  ])
  .step('finalize-order', async (ctx) => {
    console.log('✅ Finalizing order');
    return {
      completed: true,
      orderId: ctx.steps['process-order'].output.orderId,
      completedAt: new Date().toISOString(),
    };
  });

// Start the order processing workflow
(async () => {
  try {
    console.log('🚀 Starting e-commerce order processing workflow...');
    
    await cronflow.start({
      webhookServer: {
        host: '127.0.0.1',
        port: 3000,
      },
    });

    console.log('✅ Order processing workflow started successfully');
    console.log('🌐 Webhook endpoint: http://127.0.0.1:3000/webhooks/stripe');
    console.log('📋 Test with: curl -X POST http://127.0.0.1:3000/webhooks/stripe \\');
    console.log('  -H "Content-Type: application/json" \\');
    console.log('  -H "stripe-signature: test_signature" \\');
    console.log('  -d \'{"type": "checkout.session.completed", "data": {"object": {"id": "cs_test", "amount_total": 10000, "metadata": {"orderId": "ord_123", "customerEmail": "customer@example.com"}}}}\'');
  } catch (error) {
    console.error('❌ Failed to start workflow:', error);
  }
})();
```

This comprehensive examples file showcases all the major features of node-cronflow with real, working examples that users can run and modify for their own needs.
