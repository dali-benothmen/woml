import { describe, expect, test } from 'bun:test';

import { ForegroundPresentation } from '../src/foreground-presentation';
import type {
  RunPresentationV1,
  WorkflowPresentationV1,
} from '../src/terminal-presentation';

const workflow: WorkflowPresentationV1 = {
  id: 'order-processing',
  name: 'Order Processing',
  description: 'Validates and accepts an order.',
  version: '1.4.0',
  definitionHash: 'sha256:fixture',
  triggers: [
    {
      id: 'newOrder',
      type: 'webhook',
      method: 'POST',
      url: 'http://127.0.0.1:3000/webhooks/orders',
      example: "curl --request POST 'http://127.0.0.1:3000/webhooks/orders'",
    },
  ],
};

const settled: RunPresentationV1 = {
  profile: 'woml.run-presentation/v1',
  workflow,
  runId: 'run_order_42',
  trigger: { id: 'newOrder', type: 'webhook' },
  status: 'succeeded',
  admittedAt: '2026-08-14T10:00:00.000Z',
  startedAt: '2026-08-14T10:00:00.001Z',
  completedAt: '2026-08-14T10:00:00.025Z',
  durationMs: 25,
  steps: [
    {
      id: 'prepare',
      name: 'Prepare order',
      description: 'Normalize the submitted order.',
      kind: 'script',
      status: 'succeeded',
      depth: 0,
      attempts: 1,
      durationMs: 18,
      result: { orderId: 'order-42' },
    },
  ],
  summary: { succeeded: 1, failed: 0, skipped: 0, cancelled: 0, total: 1 },
  lifecycle: [],
  result: { accepted: true },
  warnings: [],
};

function triggerProgress(
  type: 'occurrence_accepted' | 'run_started' | 'run_terminal'
) {
  if (type === 'occurrence_accepted') {
    return {
      contract: 'woml.trigger-progress' as const,
      contractVersion: 1 as const,
      type,
      workflowId: workflow.id,
      triggerId: 'newOrder',
      triggerHandler: 'trigger.webhook',
      occurrenceId: 'occurrence_42',
      runId: settled.runId,
      duplicate: false,
      occurredAt: settled.admittedAt,
    };
  }
  if (type === 'run_started') {
    return {
      contract: 'woml.trigger-progress' as const,
      contractVersion: 1 as const,
      type,
      workflowId: workflow.id,
      triggerId: 'newOrder',
      triggerHandler: 'trigger.webhook',
      occurrenceId: 'occurrence_42',
      runId: settled.runId,
      occurredAt: settled.startedAt!,
    };
  }
  return {
    contract: 'woml.trigger-progress' as const,
    contractVersion: 1 as const,
    type,
    workflowId: workflow.id,
    runId: settled.runId,
    status: 'succeeded' as const,
    occurredAt: settled.completedAt!,
  };
}

describe('foreground workflow presentation', () => {
  test('prints activation, admission, and one atomic durable settlement block', () => {
    const stderrWrites: string[] = [];
    const presenter = new ForegroundPresentation({
      io: { stdout: () => {}, stderr: text => stderrWrites.push(text) },
      render: { format: 'plain', color: 'never', width: 72, timeZone: 'UTC' },
      verbose: false,
      inspectRun: () => settled,
    });

    presenter.startup(workflow);
    presenter.trigger(triggerProgress('occurrence_accepted'));
    presenter.trigger(triggerProgress('run_started'));
    presenter.trigger(triggerProgress('run_terminal'));
    presenter.trigger(triggerProgress('run_terminal'));

    expect(stderrWrites).toHaveLength(3);
    expect(stderrWrites[0]).toContain('Order Processing');
    expect(stderrWrites[0]).toContain('POST  http://127.0.0.1:3000/webhooks/orders');
    expect(stderrWrites[1]).toContain('RUN  run_order_42');
    expect(stderrWrites[1]).toContain('Accepted · webhook · newOrder');
    expect(stderrWrites[2]).toContain('Prepare order');
    expect(stderrWrites[2]).toContain('{ accepted: true }');
    expect(stderrWrites[2]).toContain('RUN COMPLETED');
  });

  test('keeps queue, retry, and actionable failure notices concise', () => {
    let stderr = '';
    const presenter = new ForegroundPresentation({
      io: { stdout: () => {}, stderr: text => { stderr += text; } },
      render: { format: 'plain', color: 'never' },
      verbose: false,
      inspectRun: () => settled,
    });
    presenter.execution({
      profile: 'woml.runtime-policy-progress/v1',
      runId: settled.runId,
      workflowId: workflow.id,
      phase: 'queued',
      queue: 'orders',
      waitingFor: 'concurrency',
    });
    presenter.execution({
      contract: 'woml.execution-progress',
      version: 1,
      type: 'step_retry_scheduled',
      runId: settled.runId,
      nodeId: 'prepare',
      nextAttempt: 2,
      maxAttempts: 3,
      scheduledAt: '2026-08-14T10:00:01.000Z',
    });
    presenter.schedule({
      contract: 'woml.schedule-progress',
      contractVersion: 1,
      type: 'scheduler_error',
      workflowId: workflow.id,
      triggerId: 'nightly',
      code: 'WOML_SCHEDULE_FAILED',
      message: 'The schedule could not advance.',
      occurredAt: '2026-08-14T10:00:00.000Z',
    });

    expect(stderr).toContain('queued · Waiting for concurrency');
    expect(stderr).toContain('retrying · Step prepare will retry (2/3)');
    expect(stderr).toContain('WOML_SCHEDULE_FAILED · The schedule could not advance.');
    expect(stderr).not.toContain('[woml:verbose]');
  });

  test('shows bounded live for-each progress without dumping item payloads', () => {
    let stderr = '';
    const presenter = new ForegroundPresentation({
      io: { stdout: () => {}, stderr: text => { stderr += text; } },
      render: { format: 'plain', color: 'never' },
      verbose: false,
      inspectRun: () => settled,
    });
    presenter.execution({
      contract: 'woml.execution-progress',
      version: 1,
      type: 'for_each_progress',
      runId: settled.runId,
      forEachId: 'organize',
      status: 'running',
      total: 42,
      succeeded: 18,
      failed: 0,
      skipped: 0,
      active: 4,
      pending: 20,
      concurrency: 4,
    });

    expect(stderr).toContain('For each organize · 18/42 completed · 4 active');
    expect(stderr).not.toContain('context');
    expect(stderr).not.toContain('items');
  });

  test('writes strict newline-delimited records to stdout in JSON mode', () => {
    let stdout = '';
    let stderr = '';
    const presenter = new ForegroundPresentation({
      io: {
        stdout: text => { stdout += text; },
        stderr: text => { stderr += text; },
      },
      render: { format: 'json', color: 'always' },
      verbose: false,
      inspectRun: () => settled,
    });

    presenter.startup(workflow);
    presenter.trigger(triggerProgress('occurrence_accepted'));
    presenter.trigger(triggerProgress('run_terminal'));

    const records = stdout.trim().split('\n').map(line => JSON.parse(line));
    expect(records).toHaveLength(3);
    expect(records[0].id).toBe(workflow.id);
    expect(records[1].profile).toBe('woml.run-presentation/v1');
    expect(records[2].profile).toBe('woml.run-presentation/v1');
    expect(stdout).not.toContain('\u001b[');
    expect(stderr).toBe('');
  });
});
