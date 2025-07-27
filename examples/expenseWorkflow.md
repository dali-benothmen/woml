# Intelligent Expense Management Workflow

AI-powered expense processing with smart approval routing and fraud detection

```typescript
import { cronflow } from 'cronflow';
import express from 'express';
import { z } from 'zod';

const app = express();
app.use(express.json());

// Types for our expense management system
interface ExpenseData {
  id: string;
  amount: number;
  description: string;
  category?: string;
  vendor?: string;
  date: string;
  receipt?: {
    url: string;
    text?: string;
    confidence?: number;
  };
  employee: {
    id: string;
    name: string;
    department: string;
    level: 'junior' | 'senior' | 'manager' | 'director';
  };
  project?: string;
  taxDeductible?: boolean;
  riskScore?: number;
}

interface ApprovalData {
  approved: boolean;
  reason?: string;
  approver?: string;
  timestamp: string;
  conditions?: string[];
}

interface Approver {
  id: string;
  name: string;
  role: string;
  required: boolean;
}

interface AIAnalysis {
  text: string;
  amount: number;
  vendor: string;
  date: string;
  category: string;
  confidence: number;
  taxDeductible: boolean;
}

// Mock AI services (in real implementation, these would call external APIs)
async function analyzeReceiptWithAI(receiptUrl: string): Promise<AIAnalysis> {
  // Simulate AI OCR and analysis
  return {
    text: 'RECEIPT - Restaurant ABC - $45.50 - Business Lunch',
    amount: 45.5,
    vendor: 'Restaurant ABC',
    date: new Date().toISOString(),
    category: 'meals',
    confidence: 0.95,
    taxDeductible: true,
  };
}

async function detectFraud(expense: ExpenseData): Promise<{
  riskScore: number;
  flags: string[];
  recommendation: 'approve' | 'review' | 'reject';
}> {
  const flags: string[] = [];
  let riskScore = 0;

  // Check for suspicious patterns
  if (expense.amount > 500) {
    riskScore += 0.3;
    flags.push('High amount');
  }

  if (expense.receipt?.confidence && expense.receipt.confidence < 0.8) {
    riskScore += 0.4;
    flags.push('Low OCR confidence');
  }

  // Weekend submission check
  const submissionDate = new Date(expense.date);
  if (submissionDate.getDay() === 0 || submissionDate.getDay() === 6) {
    riskScore += 0.2;
    flags.push('Weekend submission');
  }

  // Duplicate detection (simplified)
  if (Math.random() < 0.1) {
    // 10% chance of "duplicate"
    riskScore += 0.6;
    flags.push('Potential duplicate');
  }

  const recommendation =
    riskScore > 0.7 ? 'reject' : riskScore > 0.4 ? 'review' : 'approve';

  return { riskScore, flags, recommendation };
}

async function predictBudgetImpact(expense: ExpenseData): Promise<{
  departmentBudgetUsed: number;
  projectBudgetUsed?: number;
  monthlyTrend: 'increasing' | 'stable' | 'decreasing';
  recommendation: string;
}> {
  // Simulate budget analysis
  return {
    departmentBudgetUsed: 0.75, // 75% used
    projectBudgetUsed: expense.project ? 0.6 : undefined,
    monthlyTrend: 'increasing',
    recommendation:
      'Budget usage is trending high this month. Consider review.',
  };
}

async function getApprovalWorkflow(expense: ExpenseData): Promise<{
  approvers: Approver[];
  autoApprove: boolean;
  escalationRules: string[];
}> {
  const approvers: Approver[] = [];
  let autoApprove = false;
  const escalationRules: string[] = [];

  // Auto-approval for small amounts
  if (expense.amount < 50 && expense.riskScore! < 0.3) {
    autoApprove = true;
  } else {
    // Manager approval required
    approvers.push({
      id: 'manager_001',
      name: 'Department Manager',
      role: 'manager',
      required: true,
    });

    // Finance approval for high amounts
    if (expense.amount > 1000) {
      approvers.push({
        id: 'finance_001',
        name: 'Finance Director',
        role: 'finance',
        required: true,
      });
      escalationRules.push('Finance approval required for amounts > $1000');
    }

    // CEO approval for very high amounts
    if (expense.amount > 5000) {
      approvers.push({
        id: 'ceo_001',
        name: 'CEO',
        role: 'executive',
        required: true,
      });
      escalationRules.push('Executive approval required for amounts > $5000');
    }
  }

  return { approvers, autoApprove, escalationRules };
}

async function sendNotification(
  type: string,
  data: any,
  recipients: string[]
): Promise<void> {
  console.log(`📧 Sending ${type} notification to:`, recipients);
  console.log('Data:', data);
  // In real implementation, integrate with email/Slack/Teams
}

async function updateExpenseDatabase(
  expense: ExpenseData,
  status: string
): Promise<void> {
  console.log(`💾 Updating expense ${expense.id} with status: ${status}`);
  // In real implementation, update your database
}

// Define the main expense management workflow
const expenseWorkflow = cronflow.define({
  id: 'intelligent-expense-management',
  name: 'Intelligent Expense Management Workflow',
  description:
    'AI-powered expense processing with smart approval routing and fraud detection',
  hooks: {
    onSuccess: (ctx, stepId) => {
      if (!stepId) {
        console.log('🎉 Expense workflow completed successfully!');
        console.log('Final result:', ctx.last);
      }
    },
    onFailure: (ctx, stepId) => {
      console.log(`❌ Expense workflow failed at step: ${stepId}`);
      console.log('Error:', ctx.step_error || ctx.error);
    },
  },
});

// Webhook endpoint for expense submissions
expenseWorkflow
  .onWebhook('/api/expenses/submit', {
    method: 'POST',
    schema: z.object({
      amount: z.number().positive(),
      description: z.string().min(1),
      receiptUrl: z.string().url().optional(),
      category: z
        .enum(['meals', 'travel', 'supplies', 'software', 'training', 'other'])
        .optional(),
      employee: z.object({
        id: z.string(),
        name: z.string(),
        department: z.string(),
        level: z.enum(['junior', 'senior', 'manager', 'director']),
      }),
      project: z.string().optional(),
    }),
  })

  // Step 1: Initial data processing and receipt analysis
  .step('process-receipt', async ctx => {
    console.log('📄 Processing expense receipt...');

    const expenseId = `exp_${Date.now()}`;
    let aiAnalysis: AIAnalysis | null = null;

    if (ctx.payload.receiptUrl) {
      try {
        aiAnalysis = await analyzeReceiptWithAI(ctx.payload.receiptUrl);
        console.log('🤖 AI Receipt Analysis:', aiAnalysis);
      } catch (error) {
        console.log('⚠️ AI analysis failed, proceeding with manual data');
      }
    }

    const expense: ExpenseData = {
      id: expenseId,
      amount: aiAnalysis?.amount || ctx.payload.amount,
      description: ctx.payload.description,
      category: aiAnalysis?.category || ctx.payload.category,
      vendor: aiAnalysis?.vendor,
      date: aiAnalysis?.date || new Date().toISOString(),
      receipt: ctx.payload.receiptUrl
        ? {
            url: ctx.payload.receiptUrl,
            text: aiAnalysis?.text,
            confidence: aiAnalysis?.confidence,
          }
        : undefined,
      employee: ctx.payload.employee,
      project: ctx.payload.project,
      taxDeductible: aiAnalysis?.taxDeductible,
    };

    return { expense, aiAnalysis };
  })

  // Step 2: Fraud detection and risk analysis
  .step('fraud-detection', async ctx => {
    console.log('🔍 Running fraud detection...');

    const fraudAnalysis = await detectFraud(ctx.last.expense);
    console.log('🚨 Fraud Analysis:', fraudAnalysis);

    // Update expense with risk score
    const expense = {
      ...ctx.last.expense,
      riskScore: fraudAnalysis.riskScore,
    };

    return { expense, fraudAnalysis };
  })

  // Step 3: Budget impact analysis
  .step('budget-analysis', async ctx => {
    console.log('💰 Analyzing budget impact...');

    const budgetAnalysis = await predictBudgetImpact(ctx.last.expense);
    console.log('📊 Budget Analysis:', budgetAnalysis);

    return {
      expense: ctx.last.expense,
      fraudAnalysis: ctx.last.fraudAnalysis,
      budgetAnalysis,
    };
  })

  // Step 4: Determine approval workflow
  .step('approval-routing', async ctx => {
    console.log('📋 Determining approval workflow...');

    const approvalWorkflow = await getApprovalWorkflow(ctx.last.expense);
    console.log('👥 Approval Workflow:', approvalWorkflow);

    return {
      expense: ctx.last.expense,
      fraudAnalysis: ctx.last.fraudAnalysis,
      budgetAnalysis: ctx.last.budgetAnalysis,
      approvalWorkflow,
    };
  })

  // Conditional: Auto-approve or require manual approval
  .if('requires-approval', ctx => !ctx.last.approvalWorkflow.autoApprove)

  // Send approval notifications
  .action('send-approval-notifications', async ctx => {
    const approvers = ctx.last.approvalWorkflow.approvers.map(a => a.id);
    await sendNotification(
      'approval_request',
      {
        expense: ctx.last.expense,
        fraudAnalysis: ctx.last.fraudAnalysis,
        budgetAnalysis: ctx.last.budgetAnalysis,
      },
      approvers
    );
  })

  // Human in the loop for approval
  .humanInTheLoop({
    timeout: '48h', // 48 hours to approve
    description: 'Expense approval required',
    onPause: (ctx, token) => {
      console.log(`⏸️ Expense ${ctx.last.expense.id} awaiting approval`);
      console.log(`🔑 Approval Token: ${token}`);
      console.log(`💵 Amount: $${ctx.last.expense.amount}`);
      console.log(`👤 Employee: ${ctx.last.expense.employee.name}`);
      console.log(`🚨 Risk Score: ${ctx.last.fraudAnalysis.riskScore}`);

      // In real implementation, send this token via email/Slack
    },
  })

  // Process approval response
  .step('process-approval', async ctx => {
    console.log('✅ Processing approval response...');

    if (ctx.last.timedOut) {
      return {
        approved: false,
        reason: 'Approval timeout',
        status: 'rejected',
      };
    }

    const approval: ApprovalData = {
      approved: ctx.last.approved,
      reason: ctx.last.reason,
      approver: ctx.last.approver,
      timestamp: new Date().toISOString(),
      conditions: ctx.last.conditions,
    };

    return {
      approval,
      status: approval.approved ? 'approved' : 'rejected',
    };
  })

  .endIf()

  // Auto-approval branch
  .if('auto-approved', ctx => ctx.last.approvalWorkflow?.autoApprove)
  .step('auto-approve', async ctx => {
    console.log('⚡ Auto-approving expense...');

    const approval: ApprovalData = {
      approved: true,
      reason: 'Auto-approved based on policy',
      timestamp: new Date().toISOString(),
    };

    return {
      approval,
      status: 'approved',
    };
  })
  .endIf()

  // Step 5: Final processing and database update
  .step('finalize-expense', async ctx => {
    console.log('🏁 Finalizing expense processing...');

    const finalStatus = ctx.last.status || 'approved';
    const expense = ctx.last.expense;
    const approval = ctx.last.approval;

    // Update database
    await updateExpenseDatabase(expense, finalStatus);

    // Prepare final result
    const result = {
      expenseId: expense.id,
      status: finalStatus,
      amount: expense.amount,
      employee: expense.employee.name,
      approval,
      fraudAnalysis: ctx.last.fraudAnalysis,
      budgetAnalysis: ctx.last.budgetAnalysis,
      processedAt: new Date().toISOString(),
    };

    return result;
  })

  // Parallel background actions
  .parallel([
    // Tax optimization analysis
    async ctx => {
      if (ctx.last.expense?.taxDeductible) {
        console.log('📈 Running tax optimization analysis...');
        // Simulate tax calculation
        const taxSavings = ctx.last.expense.amount * 0.25; // 25% tax rate
        return { taxSavings, optimized: true };
      }
      return { taxSavings: 0, optimized: false };
    },

    // Update reporting dashboard
    async ctx => {
      console.log('📊 Updating expense analytics dashboard...');
      // Simulate dashboard update
      return { dashboardUpdated: true, timestamp: new Date().toISOString() };
    },

    // Integration with accounting system
    async ctx => {
      if (ctx.last.status === 'approved') {
        console.log('🔗 Syncing with accounting system...');
        // Simulate accounting integration
        return { accountingSync: true, referenceNumber: `ACC_${Date.now()}` };
      }
      return { accountingSync: false };
    },
  ])

  // Final notifications
  .action('send-final-notifications', async ctx => {
    const expense = ctx.last.expense;
    const status = ctx.last.status;

    // Notify employee
    await sendNotification(
      'expense_processed',
      {
        expenseId: expense.id,
        status,
        amount: expense.amount,
      },
      [expense.employee.id]
    );

    // Notify finance if approved
    if (status === 'approved') {
      await sendNotification('expense_approved', ctx.last, ['finance_team']);
    }
  });

// Additional webhook for approval responses (if using external approval system)
const approvalResponseWorkflow = cronflow.define({
  id: 'expense-approval-response',
  name: 'Expense Approval Response Handler',
  description: 'Handle external approval responses',
});

approvalResponseWorkflow
  .onWebhook('/api/expenses/approve', {
    method: 'POST',
    schema: z.object({
      token: z.string(),
      approved: z.boolean(),
      reason: z.string().optional(),
      approver: z.string(),
      conditions: z.array(z.string()).optional(),
    }),
  })
  .step('process-external-approval', async ctx => {
    console.log('📥 Processing external approval response...');

    try {
      // Resume the paused workflow
      await cronflow.resume(ctx.payload.token, {
        approved: ctx.payload.approved,
        reason: ctx.payload.reason,
        approver: ctx.payload.approver,
        conditions: ctx.payload.conditions,
      });

      return { success: true, message: 'Approval processed successfully' };
    } catch (error) {
      console.error('Failed to process approval:', error);
      return { success: false, error: (error as Error).message };
    }
  });

// Utility workflow for expense reporting and analytics
const expenseReportingWorkflow = cronflow.define({
  id: 'expense-reporting',
  name: 'Expense Reporting and Analytics',
  description: 'Generate expense reports and analytics',
});

expenseReportingWorkflow
  .onWebhook('/api/expenses/report', {
    method: 'GET',
    schema: z.object({
      department: z.string().optional(),
      dateRange: z
        .object({
          start: z.string(),
          end: z.string(),
        })
        .optional(),
      employeeId: z.string().optional(),
    }),
  })
  .step('generate-report', async ctx => {
    console.log('📋 Generating expense report...');

    // Simulate report generation
    const report = {
      totalExpenses: 15420.5,
      approvedExpenses: 14890.3,
      pendingExpenses: 530.2,
      rejectedExpenses: 0,
      topCategories: [
        { category: 'meals', amount: 5420.5 },
        { category: 'travel', amount: 4890.3 },
        { category: 'supplies', amount: 3120.4 },
      ],
      fraudDetected: 2,
      avgProcessingTime: '4.2 hours',
      generatedAt: new Date().toISOString(),
    };

    return { report, success: true };
  });

// Express Routes
app.get('/', (req, res) => {
  res.json({
    service: 'Intelligent Expense Management',
    status: 'ACTIVE',
    features: [
      'AI-powered receipt analysis and OCR',
      'Smart fraud detection and risk scoring',
      'Automated approval routing based on policies',
      'Budget impact analysis and forecasting',
      'Real-time expense tracking and reporting',
    ],
    advantages: [
      'Reduces manual processing by 80%',
      'Catches fraud before it costs money',
      'Ensures policy compliance automatically',
      'Provides real-time expense insights',
      'Integrates with existing accounting systems',
    ],
  });
});

// Submit expense endpoint
app.post('/api/expenses/submit', async (req, res) => {
  try {
    const expenseData = {
      amount: req.body.amount,
      description: req.body.description,
      receiptUrl: req.body.receiptUrl,
      category: req.body.category,
      employee: req.body.employee,
      project: req.body.project,
    };

    console.log('\n📄 EXPENSE SUBMITTED');
    console.log(`   Amount: $${expenseData.amount}`);
    console.log(`   Employee: ${expenseData.employee.name}`);
    console.log(`   Category: ${expenseData.category || 'Not specified'}`);

    const runId = await cronflow.trigger(
      'intelligent-expense-management',
      expenseData
    );

    res.json({
      success: true,
      expenseId: `exp_${Date.now()}`,
      workflowRunId: runId,
      message: 'Expense submitted for processing',
      estimatedProcessingTime: '2-5 minutes',
      trackingUrl: `/api/expenses/${runId}/status`,
    });
  } catch (error) {
    console.error('Expense submission error:', error);
    res.status(400).json({
      success: false,
      error: (error as Error).message || 'Failed to submit expense',
    });
  }
});

// Approve expense endpoint
app.post('/api/expenses/approve', async (req, res) => {
  try {
    const approvalData = {
      token: req.body.token,
      approved: req.body.approved,
      reason: req.body.reason,
      approver: req.body.approver,
      conditions: req.body.conditions,
    };

    console.log('\n✅ EXPENSE APPROVAL RECEIVED');
    console.log(`   Token: ${approvalData.token}`);
    console.log(`   Approved: ${approvalData.approved}`);
    console.log(`   Approver: ${approvalData.approver}`);

    const runId = await cronflow.trigger(
      'expense-approval-response',
      approvalData
    );

    res.json({
      success: true,
      message: 'Approval processed successfully',
      workflowRunId: runId,
    });
  } catch (error) {
    console.error('Approval processing error:', error);
    res.status(400).json({
      success: false,
      error: (error as Error).message || 'Failed to process approval',
    });
  }
});

// Get expense report endpoint
app.get('/api/expenses/report', async (req, res) => {
  try {
    const reportParams = {
      department: req.query.department as string,
      dateRange:
        req.query.dateRange && typeof req.query.dateRange === 'object'
          ? {
              start: (req.query.dateRange as any).start as string,
              end: (req.query.dateRange as any).end as string,
            }
          : undefined,
      employeeId: req.query.employeeId as string,
    };

    console.log('\n📊 EXPENSE REPORT REQUESTED');
    console.log(`   Department: ${reportParams.department || 'All'}`);
    console.log(
      `   Date Range: ${reportParams.dateRange ? `${reportParams.dateRange.start} to ${reportParams.dateRange.end}` : 'All time'}`
    );

    const runId = await cronflow.trigger('expense-reporting', reportParams);

    res.json({
      success: true,
      message: 'Report generation started',
      workflowRunId: runId,
      estimatedCompletion: '30-60 seconds',
    });
  } catch (error) {
    console.error('Report generation error:', error);
    res.status(400).json({
      success: false,
      error: (error as Error).message || 'Failed to generate report',
    });
  }
});

// Get expense status endpoint
app.get('/api/expenses/:runId/status', (req, res) => {
  const { runId } = req.params;

  // Mock expense status
  const status = {
    runId,
    status: 'COMPLETED',
    expenseId: `exp_${Date.now()}`,
    amount: 245.5,
    employee: 'John Doe',
    category: 'meals',
    approvalStatus: 'APPROVED',
    fraudScore: 0.15,
    processingTime: '3.2 minutes',
    lastUpdated: new Date().toISOString(),
    nextSteps: 'Payment will be processed within 2 business days',
  };

  res.json({
    success: true,
    status,
  });
});

// Health check endpoint
app.get('/api/health', (req, res) => {
  res.json({
    status: 'HEALTHY',
    timestamp: new Date().toISOString(),
    services: {
      cronflow: 'ACTIVE',
      expense_processing: 'READY',
      ai_analysis: 'AVAILABLE',
      fraud_detection: 'OPERATIONAL',
    },
    performance: {
      activeExpenses: 12,
      avgProcessingTime: '3.2 minutes',
      approvalRate: '94.2%',
      fraudDetectionRate: '98.7%',
      uptime: '99.96%',
    },
  });
});

// Start server
app.listen(3001, async () => {
  console.log('\n💰 Intelligent Expense Management Starting...');
  console.log('⚡ Server running on port 3001');

  await cronflow.start();

  console.log('\n✅ Intelligent Expense Management - READY!');
  console.log('\n🎯 Key Features:');
  console.log('   ✅ AI-powered receipt analysis and OCR');
  console.log('   ✅ Smart fraud detection and risk scoring');
  console.log('   ✅ Automated approval routing based on policies');
  console.log('   ✅ Budget impact analysis and forecasting');
  console.log('   ✅ Real-time expense tracking and reporting');
  console.log('\n📋 Endpoints:');
  console.log('   POST /api/expenses/submit - Submit new expense');
  console.log('   POST /api/expenses/approve - Approve expense');
  console.log('   GET /api/expenses/report - Generate expense report');
  console.log('   GET /api/expenses/:id/status - Check expense status');
  console.log('\n💪 Ready for intelligent expense management!');
});

// Export the workflows
export { expenseWorkflow, approvalResponseWorkflow, expenseReportingWorkflow };

/* 
USAGE EXAMPLES:

1. Submit an expense:
curl -X POST http://localhost:3001/api/expenses/submit \
  -H "Content-Type: application/json" \
  -d '{
    "amount": 245.50,
    "description": "Business lunch with client",
    "receiptUrl": "https://example.com/receipt.jpg",
    "category": "meals",
    "employee": {
      "id": "emp_123",
      "name": "John Doe",
      "department": "Sales",
      "level": "senior"
    },
    "project": "Client Acquisition"
  }'

2. Approve an expense:
curl -X POST http://localhost:3001/api/expenses/approve \
  -H "Content-Type: application/json" \
  -d '{
    "token": "approval_token_123",
    "approved": true,
    "reason": "Valid business expense",
    "approver": "manager_001"
  }'

3. Generate expense report:
curl "http://localhost:3001/api/expenses/report?department=Sales&dateRange[start]=2025-01-01&dateRange[end]=2025-01-31"

FEATURES:
✅ AI-powered receipt analysis and OCR
✅ Smart fraud detection and risk scoring
✅ Automated approval routing based on policies
✅ Budget impact analysis and forecasting
✅ Real-time expense tracking and reporting
✅ Human-in-the-loop approval system
✅ Tax optimization analysis
✅ Integration with accounting systems
✅ Comprehensive expense analytics
✅ Type-safe configuration with Zod validation
*/
```
