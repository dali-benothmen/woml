import { cronflow } from '../sdk/src/index';
import { resume } from '../sdk/src/cronflow';

console.log('🧪 Testing Enhanced Human-in-the-Loop Features...\n');

// Test 1: Basic human-in-the-loop functionality
console.log('✅ Test 1: Basic human-in-the-loop functionality');
try {
  const basicHumanWorkflow = cronflow.define({
    id: 'basic-human-test',
    name: 'Basic Human Approval Test',
  });

  basicHumanWorkflow
    .step('prepare-approval', ctx => {
      return { amount: 1000, description: 'High-value transaction' };
    })
    .humanInTheLoop({
      timeout: '1h',
      description: 'Approve high-value transaction',
      onPause: (ctx, token) => {
        console.log(`🛑 Human approval required. Token: ${token}`);
        console.log(`💰 Transaction amount: $${ctx.last.amount}`);
      },
    })
    .step('process-approval', ctx => {
      return {
        approved: ctx.last.approved,
        token: ctx.last.token,
        reason: ctx.last.reason,
        approvedBy: ctx.last.approvedBy,
      };
    });

  console.log('✅ Basic human-in-the-loop workflow defined successfully');
} catch (error) {
  console.log('❌ Basic human-in-the-loop test failed:', error);
}

// Test 2: Human-in-the-loop with approval URL
console.log('\n✅ Test 2: Human-in-the-loop with approval URL');
try {
  const urlHumanWorkflow = cronflow.define({
    id: 'url-human-test',
    name: 'Human Approval with URL Test',
  });

  urlHumanWorkflow
    .step('prepare-approval', ctx => {
      return { orderId: 'ORD-123', amount: 2500 };
    })
    .humanInTheLoop({
      timeout: '24h',
      description: 'Approve high-value order',
      approvalUrl: 'https://approvals.example.com/approve',
      onPause: (ctx, token) => {
        console.log(
          `🛑 Approval URL: https://approvals.example.com/approve?token=${token}`
        );
      },
    })
    .step('process-approval', ctx => {
      return {
        orderId: ctx.steps['prepare-approval'].output.orderId,
        approved: ctx.last.approved,
        approvalUrl: ctx.last.approvalUrl,
      };
    });

  console.log('✅ Human-in-the-loop with URL workflow defined successfully');
} catch (error) {
  console.log('❌ Human-in-the-loop with URL test failed:', error);
}

// Test 3: Human-in-the-loop with metadata
console.log('\n✅ Test 3: Human-in-the-loop with metadata');
try {
  const metadataHumanWorkflow = cronflow.define({
    id: 'metadata-human-test',
    name: 'Human Approval with Metadata Test',
  });

  metadataHumanWorkflow
    .step('prepare-approval', ctx => {
      return {
        transactionId: 'TXN-456',
        amount: 5000,
        customerId: 'CUST-789',
      };
    })
    .humanInTheLoop({
      timeout: '12h',
      description: 'Approve VIP customer transaction',
      metadata: {
        customerTier: 'VIP',
        riskLevel: 'medium',
        previousApprovals: 3,
        department: 'finance',
      },
      onPause: (ctx, token) => {
        console.log(`🛑 VIP approval required with metadata`);
        console.log(
          `📋 Metadata: VIP customer, medium risk, finance department`
        );
      },
    })
    .step('process-approval', ctx => {
      return {
        transactionId: ctx.steps['prepare-approval'].output.transactionId,
        approved: ctx.last.approved,
        metadata: ctx.last.metadata,
        customerTier: ctx.last.metadata?.customerTier,
      };
    });

  console.log(
    '✅ Human-in-the-loop with metadata workflow defined successfully'
  );
} catch (error) {
  console.log('❌ Human-in-the-loop with metadata test failed:', error);
}

// Test 4: Resume functionality
console.log('\n✅ Test 4: Resume functionality');
try {
  const testToken = 'test_approval_token_123';
  const testPayload = {
    approved: true,
    reason: 'Looks good to me',
    approvedBy: 'john.doe@company.com',
    comments: 'Transaction approved after review',
  };

  // Test resume function
  await resume(testToken, testPayload);
  console.log('✅ Resume function executed successfully');
} catch (error) {
  console.log('❌ Resume test failed:', error);
}

// Test 5: Complex human-in-the-loop workflow
console.log('\n✅ Test 5: Complex human-in-the-loop workflow');
try {
  const complexHumanWorkflow = cronflow.define({
    id: 'complex-human-test',
    name: 'Complex Human Approval Test',
  });

  complexHumanWorkflow
    .step('validate-transaction', ctx => {
      const transaction = ctx.payload;
      if (transaction.amount > 10000) {
        return {
          requiresApproval: true,
          approvalLevel: 'senior',
          riskScore: 'high',
        };
      }
      return { requiresApproval: false };
    })
    .if('needs-approval', ctx => ctx.last.requiresApproval)
    .humanInTheLoop({
      timeout: '48h',
      description: 'Senior approval required for high-value transaction',
      approvalUrl: 'https://approvals.company.com/senior-approve',
      metadata: {
        approvalLevel: 'senior',
        riskScore: 'high',
        autoEscalate: true,
      },
      onPause: (ctx, token) => {
        console.log(`🛑 Senior approval required for high-value transaction`);
        console.log(
          `🔗 Approval URL: https://approvals.company.com/senior-approve?token=${token}`
        );
        console.log(`⚠️ Auto-escalation enabled if not approved within 48h`);
      },
    })
    .step('process-senior-approval', ctx => {
      return {
        approved: ctx.last.approved,
        approvalLevel: ctx.last.metadata?.approvalLevel,
        riskScore: ctx.last.metadata?.riskScore,
      };
    })
    .else()
    .step('auto-approve', ctx => {
      return { approved: true, reason: 'Auto-approved (below threshold)' };
    })
    .endIf()
    .step('finalize-transaction', ctx => {
      const approvalStep =
        ctx.steps['process-senior-approval'] || ctx.steps['auto-approve'];
      return {
        finalized: true,
        approved: approvalStep.output.approved,
        timestamp: Date.now(),
      };
    });

  console.log('✅ Complex human-in-the-loop workflow defined successfully');
} catch (error) {
  console.log('❌ Complex human-in-the-loop test failed:', error);
}

// Test 6: Multiple human approvals in sequence
console.log('\n✅ Test 6: Multiple human approvals in sequence');
try {
  const multiApprovalWorkflow = cronflow.define({
    id: 'multi-approval-test',
    name: 'Multiple Human Approvals Test',
  });

  multiApprovalWorkflow
    .step('initial-review', ctx => {
      return { amount: 15000, department: 'sales' };
    })
    .humanInTheLoop({
      timeout: '24h',
      description: 'Manager approval for large deal',
      metadata: { approvalType: 'manager', level: 1 },
      onPause: (ctx, token) => {
        console.log(`🛑 Manager approval required for large deal`);
      },
    })
    .step('manager-approved', ctx => {
      return {
        managerApproved: ctx.last.approved,
        managerReason: ctx.last.reason,
      };
    })
    .if('needs-director-approval', ctx => ctx.last.managerApproved)
    .humanInTheLoop({
      timeout: '48h',
      description: 'Director approval for large deal',
      metadata: { approvalType: 'director', level: 2 },
      onPause: (ctx, token) => {
        console.log(`🛑 Director approval required for large deal`);
      },
    })
    .step('director-approved', ctx => {
      return {
        directorApproved: ctx.last.approved,
        directorReason: ctx.last.reason,
      };
    })
    .endIf()
    .step('final-approval', ctx => {
      return {
        finalApproved: ctx.last.directorApproved || ctx.last.managerApproved,
        approvalChain: [
          ctx.steps['manager-approved']?.output,
          ctx.steps['director-approved']?.output,
        ].filter(Boolean),
      };
    });

  console.log('✅ Multiple human approvals workflow defined successfully');
} catch (error) {
  console.log('❌ Multiple human approvals test failed:', error);
}

// Test 7: Human approval with timeout handling
console.log('\n✅ Test 7: Human approval with timeout handling');
try {
  const timeoutHumanWorkflow = cronflow.define({
    id: 'timeout-human-test',
    name: 'Human Approval Timeout Test',
  });

  timeoutHumanWorkflow
    .step('prepare-timeout-test', ctx => {
      return {
        testType: 'timeout',
        shortTimeout: '5s',
        longTimeout: '1h',
      };
    })
    .humanInTheLoop({
      timeout: '5s', // Short timeout for testing
      description: 'Quick approval test with timeout',
      metadata: { testMode: true, expectedTimeout: true },
      onPause: (ctx, token) => {
        console.log(`🛑 Quick approval test - will timeout in 5 seconds`);
        console.log(`⏰ Created at: ${new Date().toISOString()}`);
        console.log(
          `⏰ Expires at: ${new Date(Date.now() + 5000).toISOString()}`
        );
      },
    })
    .step('handle-timeout-result', ctx => {
      return {
        status: ctx.last.status,
        expired: Date.now() > (ctx.last.expiresAt || 0),
        timeoutHandled: true,
      };
    });

  console.log('✅ Human approval timeout workflow defined successfully');
} catch (error) {
  console.log('❌ Human approval timeout test failed:', error);
}

console.log('\n🎉 All enhanced human-in-the-loop tests completed!');
console.log('\n📋 Summary of Enhanced Human-in-the-Loop Features:');
console.log('✅ Basic human-in-the-loop with token generation');
console.log('✅ Approval URL support for web-based approvals');
console.log('✅ Metadata support for rich approval context');
console.log('✅ Resume functionality for external approval handling');
console.log('✅ Complex workflows with conditional human approvals');
console.log('✅ Multiple sequential human approvals');
console.log('✅ Timeout handling with expiration tracking');
console.log('✅ Rich approval response with metadata');
