# Human-in-the-Loop

Cronflow provides powerful human-in-the-loop capabilities that allow you to pause workflows for manual approval, review, or intervention. This is perfect for workflows that require human oversight, such as approval processes, content moderation, or complex decision-making.

## Overview

Human-in-the-loop workflows allow you to:

- **Pause workflows** for manual review
- **Request approvals** from specific users or roles
- **Resume workflows** after human intervention
- **Set timeouts** for approval requests
- **Track approval status** and history

## Basic Human-in-the-Loop

### Simple Approval Workflow

```typescript
const approvalWorkflow = cronflow.define({
  id: 'approval-workflow',
  name: 'Approval Workflow',
});

approvalWorkflow
  .step('submit-request', async ctx => {
    return await createRequest(ctx.payload);
  })
  .humanInTheLoop({
    token: 'approval-request-123',
    description: 'Please review and approve this request',
    approvalUrl: 'https://app.example.com/approve/approval-request-123',
    metadata: {
      requestType: 'expense',
      amount: ctx.last.amount,
      requester: ctx.last.requesterId,
    },
  })
  .step('process-approved-request', async ctx => {
    return await processApprovedRequest(ctx.last);
  });
```

### Approval with Timeout

```typescript
const timeoutApprovalWorkflow = cronflow.define({
  id: 'timeout-approval-workflow',
  name: 'Timeout Approval Workflow',
});

timeoutApprovalWorkflow
  .step('create-expense', async ctx => {
    return await createExpense(ctx.payload);
  })
  .humanInTheLoop({
    token: `expense-${ctx.last.id}`,
    description: 'Approve expense request',
    approvalUrl: `https://app.example.com/expenses/${ctx.last.id}/approve`,
    timeout: '24h', // Auto-approve after 24 hours
    metadata: {
      expenseId: ctx.last.id,
      amount: ctx.last.amount,
      category: ctx.last.category,
    },
  })
  .step('process-expense', async ctx => {
    return await processExpense(ctx.last);
  });
```

## Advanced Human-in-the-Loop Patterns

### 1. Conditional Approval

```typescript
const conditionalApprovalWorkflow = cronflow.define({
  id: 'conditional-approval-workflow',
  name: 'Conditional Approval Workflow',
});

conditionalApprovalWorkflow
  .step('process-order', async ctx => {
    return await processOrder(ctx.payload);
  })
  .if('requires-approval', ctx => ctx.last.amount > 1000)
  .humanInTheLoop({
    token: `order-approval-${ctx.last.id}`,
    description: `Order approval required for $${ctx.last.amount}`,
    approvalUrl: `https://app.example.com/orders/${ctx.last.id}/approve`,
    metadata: {
      orderId: ctx.last.id,
      amount: ctx.last.amount,
      customerId: ctx.last.customerId,
    },
  })
  .else()
  .step('auto-approve', async ctx => {
    return { autoApproved: true, order: ctx.last };
  })
  .endIf()
  .step('finalize-order', async ctx => {
    return await finalizeOrder(ctx.last);
  });
```

### 2. Multi-Level Approval

```typescript
const multiLevelApprovalWorkflow = cronflow.define({
  id: 'multi-level-approval-workflow',
  name: 'Multi-Level Approval Workflow',
});

multiLevelApprovalWorkflow
  .step('submit-request', async ctx => {
    return await submitRequest(ctx.payload);
  })
  .if('requires-manager-approval', ctx => ctx.last.amount > 5000)
  .humanInTheLoop({
    token: `manager-approval-${ctx.last.id}`,
    description: 'Manager approval required',
    approvalUrl: `https://app.example.com/manager/approve/${ctx.last.id}`,
    metadata: {
      level: 'manager',
      requestId: ctx.last.id,
      amount: ctx.last.amount,
    },
  })
  .if('requires-director-approval', ctx => ctx.last.amount > 10000)
  .humanInTheLoop({
    token: `director-approval-${ctx.last.id}`,
    description: 'Director approval required',
    approvalUrl: `https://app.example.com/director/approve/${ctx.last.id}`,
    metadata: {
      level: 'director',
      requestId: ctx.last.id,
      amount: ctx.last.amount,
    },
  })
  .endIf()
  .endIf()
  .step('process-approved-request', async ctx => {
    return await processApprovedRequest(ctx.last);
  });
```

### 3. Parallel Approvals

```typescript
const parallelApprovalWorkflow = cronflow.define({
  id: 'parallel-approval-workflow',
  name: 'Parallel Approval Workflow',
});

parallelApprovalWorkflow
  .step('create-project', async ctx => {
    return await createProject(ctx.payload);
  })
  .parallel([
    async ctx => {
      return await ctx.humanInTheLoop({
        token: `technical-approval-${ctx.last.id}`,
        description: 'Technical review required',
        approvalUrl: `https://app.example.com/technical/approve/${ctx.last.id}`,
        metadata: {
          type: 'technical',
          projectId: ctx.last.id,
        },
      });
    },
    async ctx => {
      return await ctx.humanInTheLoop({
        token: `business-approval-${ctx.last.id}`,
        description: 'Business review required',
        approvalUrl: `https://app.example.com/business/approve/${ctx.last.id}`,
        metadata: {
          type: 'business',
          projectId: ctx.last.id,
        },
      });
    },
  ])
  .step('process-project', async ctx => {
    const [technicalApproval, businessApproval] = ctx.last;
    return await processProject(ctx.steps['create-project'].output, {
      technicalApproval,
      businessApproval,
    });
  });
```

### 4. Approval with Escalation

```typescript
const escalationWorkflow = cronflow.define({
  id: 'escalation-workflow',
  name: 'Escalation Workflow',
});

escalationWorkflow
  .step('create-ticket', async ctx => {
    return await createTicket(ctx.payload);
  })
  .humanInTheLoop({
    token: `ticket-approval-${ctx.last.id}`,
    description: 'Please review this ticket',
    approvalUrl: `https://app.example.com/tickets/${ctx.last.id}/approve`,
    timeout: '4h',
    metadata: {
      ticketId: ctx.last.id,
      priority: ctx.last.priority,
      assignee: ctx.last.assigneeId,
    },
  })
  .if('escalation-needed', ctx => ctx.last.approved === false)
  .humanInTheLoop({
    token: `escalation-approval-${ctx.last.id}`,
    description: 'Escalated approval required',
    approvalUrl: `https://app.example.com/escalation/approve/${ctx.last.id}`,
    timeout: '2h',
    metadata: {
      ticketId: ctx.last.id,
      escalated: true,
      originalAssignee: ctx.last.assigneeId,
    },
  })
  .endIf()
  .step('process-ticket', async ctx => {
    return await processTicket(ctx.last);
  });
```

## Approval Management

### List Paused Workflows

```typescript
// Get all paused workflows waiting for approval
const pausedWorkflows = await cronflow.listPausedWorkflows();

console.log('Paused workflows:', pausedWorkflows);
// Output: [
//   {
//     workflowId: 'approval-workflow',
//     runId: 'run-123',
//     token: 'approval-request-123',
//     description: 'Please review and approve this request',
//     createdAt: '2024-01-01T10:00:00Z',
//     expiresAt: '2024-01-02T10:00:00Z',
//     metadata: { requestType: 'expense', amount: 500 }
//   }
// ]
```

### Get Specific Paused Workflow

```typescript
// Get details of a specific paused workflow
const pausedWorkflow = await cronflow.getPausedWorkflow('approval-request-123');

console.log('Paused workflow details:', pausedWorkflow);
// Output: {
//   workflowId: 'approval-workflow',
//   runId: 'run-123',
//   token: 'approval-request-123',
//   description: 'Please review and approve this request',
//   approvalUrl: 'https://app.example.com/approve/approval-request-123',
//   createdAt: '2024-01-01T10:00:00Z',
//   expiresAt: '2024-01-02T10:00:00Z',
//   metadata: { requestType: 'expense', amount: 500 }
// }
```

### Resume Workflow

```typescript
// Resume a paused workflow with approval decision
await cronflow.resume('approval-request-123', {
  approved: true,
  approvedBy: 'user-456',
  approvedAt: new Date().toISOString(),
  comments: 'Looks good, approved',
});

// Or reject the workflow
await cronflow.resume('approval-request-123', {
  approved: false,
  rejectedBy: 'user-456',
  rejectedAt: new Date().toISOString(),
  comments: 'Missing required documentation',
});
```

## Advanced Patterns

### 1. Approval with Comments

```typescript
const commentApprovalWorkflow = cronflow.define({
  id: 'comment-approval-workflow',
  name: 'Comment Approval Workflow',
});

commentApprovalWorkflow
  .step('submit-content', async ctx => {
    return await submitContent(ctx.payload);
  })
  .humanInTheLoop({
    token: `content-approval-${ctx.last.id}`,
    description: 'Review and approve content',
    approvalUrl: `https://app.example.com/content/${ctx.last.id}/review`,
    requireComments: true, // Require comments for approval/rejection
    metadata: {
      contentId: ctx.last.id,
      contentType: ctx.last.type,
      author: ctx.last.authorId,
    },
  })
  .step('publish-content', async ctx => {
    const approval = ctx.last;
    return await publishContent(ctx.steps['submit-content'].output, {
      approved: approval.approved,
      comments: approval.comments,
      reviewer: approval.reviewedBy,
    });
  });
```

### 2. Conditional Approval Based on Data

```typescript
const dataDrivenApprovalWorkflow = cronflow.define({
  id: 'data-driven-approval-workflow',
  name: 'Data-Driven Approval Workflow',
});

dataDrivenApprovalWorkflow
  .step('analyze-risk', async ctx => {
    return await analyzeRisk(ctx.payload);
  })
  .if('high-risk', ctx => ctx.last.riskScore > 0.8)
  .humanInTheLoop({
    token: `risk-approval-${ctx.last.id}`,
    description: 'High-risk transaction requires approval',
    approvalUrl: `https://app.example.com/risk/approve/${ctx.last.id}`,
    metadata: {
      riskScore: ctx.last.riskScore,
      riskFactors: ctx.last.riskFactors,
      transactionId: ctx.last.id,
    },
  })
  .elseIf('medium-risk', ctx => ctx.last.riskScore > 0.5)
  .humanInTheLoop({
    token: `medium-risk-approval-${ctx.last.id}`,
    description: 'Medium-risk transaction review',
    approvalUrl: `https://app.example.com/medium-risk/approve/${ctx.last.id}`,
    timeout: '12h',
    metadata: {
      riskScore: ctx.last.riskScore,
      transactionId: ctx.last.id,
    },
  })
  .endIf()
  .step('process-transaction', async ctx => {
    return await processTransaction(ctx.last);
  });
```

### 3. Approval with Notifications

```typescript
const notificationApprovalWorkflow = cronflow.define({
  id: 'notification-approval-workflow',
  name: 'Notification Approval Workflow',
});

notificationApprovalWorkflow
  .step('create-request', async ctx => {
    return await createRequest(ctx.payload);
  })
  .action('notify-approver', async ctx => {
    await emailService.send({
      to: ctx.last.approverEmail,
      subject: 'Approval Required',
      template: 'approval-request',
      data: {
        requestId: ctx.last.id,
        amount: ctx.last.amount,
        approvalUrl: `https://app.example.com/approve/${ctx.last.id}`,
      },
    });
  })
  .humanInTheLoop({
    token: `request-approval-${ctx.last.id}`,
    description: 'Please review and approve this request',
    approvalUrl: `https://app.example.com/approve/${ctx.last.id}`,
    timeout: '48h',
    metadata: {
      requestId: ctx.last.id,
      requester: ctx.last.requesterId,
      approver: ctx.last.approverId,
    },
  })
  .action('notify-requester', async ctx => {
    const approval = ctx.last;
    await emailService.send({
      to: ctx.steps['create-request'].output.requesterEmail,
      subject: approval.approved ? 'Request Approved' : 'Request Rejected',
      template: approval.approved ? 'request-approved' : 'request-rejected',
      data: {
        requestId: approval.requestId,
        approved: approval.approved,
        comments: approval.comments,
      },
    });
  });
```

### 4. Approval with Time-Based Logic

```typescript
const timeBasedApprovalWorkflow = cronflow.define({
  id: 'time-based-approval-workflow',
  name: 'Time-Based Approval Workflow',
});

timeBasedApprovalWorkflow
  .step('check-business-hours', async ctx => {
    const now = new Date();
    const hour = now.getHours();
    const isBusinessHours = hour >= 9 && hour <= 17;

    return { isBusinessHours, currentTime: now.toISOString() };
  })
  .if('business-hours', ctx => ctx.last.isBusinessHours)
  .humanInTheLoop({
    token: `business-hours-approval-${Date.now()}`,
    description: 'Approval required during business hours',
    approvalUrl: `https://app.example.com/business-hours/approve`,
    timeout: '2h',
    metadata: {
      requestTime: ctx.last.currentTime,
      businessHours: true,
    },
  })
  .else()
  .humanInTheLoop({
    token: `after-hours-approval-${Date.now()}`,
    description: 'After-hours approval required',
    approvalUrl: `https://app.example.com/after-hours/approve`,
    timeout: '8h', // Longer timeout for after-hours
    metadata: {
      requestTime: ctx.last.currentTime,
      businessHours: false,
    },
  })
  .endIf()
  .step('process-request', async ctx => {
    return await processRequest(ctx.last);
  });
```

## Best Practices

### 1. Set Appropriate Timeouts

```typescript
// Short timeout for urgent requests
.humanInTheLoop({
  token: 'urgent-approval',
  description: 'Urgent approval required',
  timeout: '1h'
})

// Medium timeout for normal requests
.humanInTheLoop({
  token: 'normal-approval',
  description: 'Standard approval required',
  timeout: '24h'
})

// Long timeout for complex decisions
.humanInTheLoop({
  token: 'complex-approval',
  description: 'Complex decision requires review',
  timeout: '72h'
})
```

### 2. Provide Clear Descriptions

```typescript
.humanInTheLoop({
  token: 'clear-approval',
  description: `Please review expense request #${ctx.last.id} for $${ctx.last.amount} from ${ctx.last.requester}. Category: ${ctx.last.category}`,
  approvalUrl: `https://app.example.com/expenses/${ctx.last.id}/approve`,
  metadata: {
    expenseId: ctx.last.id,
    amount: ctx.last.amount,
    category: ctx.last.category,
    requester: ctx.last.requester
  }
})
```

### 3. Handle Approval Outcomes

```typescript
workflow
  .humanInTheLoop({
    token: 'approval-with-outcomes',
    description: 'Review and approve',
    approvalUrl: 'https://app.example.com/approve',
  })
  .if('approved', ctx => ctx.last.approved === true)
  .step('process-approval', async ctx => {
    return await processApprovedRequest(ctx.last);
  })
  .else()
  .step('handle-rejection', async ctx => {
    return await handleRejectedRequest(ctx.last);
  })
  .endIf();
```

### 4. Track Approval History

```typescript
workflow
  .step('create-request', async ctx => {
    return await createRequest(ctx.payload);
  })
  .humanInTheLoop({
    token: `request-${ctx.last.id}`,
    description: 'Approve request',
    approvalUrl: `https://app.example.com/approve/${ctx.last.id}`,
    metadata: {
      requestId: ctx.last.id,
      createdAt: new Date().toISOString(),
    },
  })
  .action('log-approval', async ctx => {
    const approval = ctx.last;
    await auditLog.create({
      action: approval.approved ? 'request_approved' : 'request_rejected',
      requestId: approval.requestId,
      approver: approval.reviewedBy,
      timestamp: new Date().toISOString(),
      comments: approval.comments,
    });
  });
```

## Related Topics

- **[Error Handling](/guide/error-handling)** - Basic error handling and recovery
- **[Testing](/guide/testing)** - Testing workflows and components
- **[Performance](/guide/performance)** - Optimizing workflow performance
