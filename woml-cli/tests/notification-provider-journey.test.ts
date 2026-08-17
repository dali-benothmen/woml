import { afterEach, describe, expect, test } from 'bun:test';
import { Database } from 'bun:sqlite';
import { existsSync } from 'node:fs';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { compileWoml, parseWoml } from 'woml';
import {
  executeApprovalWorkflowWithRust,
  NotificationProviderError,
  resumeApprovalWorkflowWithRust,
  runNotificationProviderJourneyWithRust,
} from '../src/rust-executor';

const packageRoot = resolve(import.meta.dir, '..');
const stagedNativeCorePath = resolve(
  packageRoot,
  'dist',
  `woml-core.${process.platform}-${process.arch}.node`
);
const nativeCorePath =
  process.env.WOML_RUST_CORE_PATH ??
  (existsSync(stagedNativeCorePath) ? stagedNativeCorePath : undefined);
const nativeTest = nativeCorePath === undefined ? test.skip : test;
const scriptHostPath = resolve(packageRoot, 'src/script-host.ts');
const notificationHostPath = resolve(
  packageRoot,
  'tests/fixtures/fake-notification-provider-host.ts'
);
const missingSecretHostPath = resolve(
  packageRoot,
  'tests/fixtures/missing-secret-notification-provider-host.ts'
);
const noActionHostPath = resolve(
  packageRoot,
  'tests/fixtures/no-action-notification-provider-host.ts'
);
const crashingHostPath = resolve(
  packageRoot,
  'tests/fixtures/crashing-notification-provider-host.ts'
);
const rateLimitedHostPath = resolve(
  packageRoot,
  'tests/fixtures/rate-limited-notification-provider-host.ts'
);
const mixedOutcomeHostPath = resolve(
  packageRoot,
  'tests/fixtures/mixed-outcome-notification-provider-host.ts'
);
const multiWorkspaceHostPath = resolve(
  packageRoot,
  'tests/fixtures/multi-workspace-notification-provider-host.ts'
);
const sourcePath = resolve(
  packageRoot,
  '../woml/tests/fixtures/approval-slack.woml'
);

const environmentNames = [
  'WOML_SECRETS_PROVIDER',
  'WOML_SECRET_SLACK_BOT_TOKEN',
  'WOML_SECRET_SLACK_APP_TOKEN',
  'WOML_FAKE_SLACK_DECISION',
  'WOML_FAKE_SLACK_ACTOR_ID',
] as const;
const originalEnvironment = Object.fromEntries(
  environmentNames.map(name => [name, process.env[name]])
);

afterEach(() => {
  for (const name of environmentNames) {
    const value = originalEnvironment[name];
    if (value === undefined) delete process.env[name];
    else process.env[name] = value;
  }
});

async function workflow() {
  return compileWoml(
    parseWoml(await Bun.file(sourcePath).text(), { file: sourcePath })
  );
}

function routedWorkflow() {
  const source = `<woml>
<workflow version="0.1" id="n4-routed-approval" name="Routed Approval">
  <triggers><manual id="start" /></triggers>
  <steps>
    <approval id="review" name="Review" timeout="24h" on-timeout="reject">
      <notify>
        <slack channels="#approvals #engineering" bot-token="{{secrets.SLACK_BOT_TOKEN}}" app-token="{{secrets.SLACK_APP_TOKEN}}" />
      </notify>
      <when-approved>
        <step id="approvedRoute"><script>return { route: 'approved' };</script></step>
      </when-approved>
      <when-rejected>
        <step id="rejectedRoute"><script>return { route: 'rejected' };</script></step>
      </when-rejected>
    </approval>
    <step id="finalStatus">
      <script>return { decision: context.steps.review.decision };</script>
    </step>
  </steps>
</workflow>
</woml>`;
  return compileWoml(parseWoml(source, { file: 'n4-routed-approval.woml' }));
}

function timeoutWorkflow() {
  const source = `<woml>
<workflow version="0.1" id="n5-timeout" name="Timeout">
  <triggers><manual id="start" /></triggers>
  <steps>
    <approval id="review" name="Review" timeout="500ms" on-timeout="reject">
      <notify>
        <slack channels="#approvals #engineering" bot-token="{{secrets.SLACK_BOT_TOKEN}}" app-token="{{secrets.SLACK_APP_TOKEN}}" />
      </notify>
      <when-approved />
      <when-rejected />
    </approval>
    <step id="finalStatus"><script>return { decision: context.steps.review.decision };</script></step>
  </steps>
</workflow>
</woml>`;
  return compileWoml(parseWoml(source, { file: 'n5-timeout.woml' }));
}

function multiWorkspaceWorkflow() {
  const source = `<woml>
<workflow version="0.1" id="n6-multi-workspace" name="Multi Workspace">
  <triggers><manual id="start" /></triggers>
  <steps>
    <approval id="review" name="Review" timeout="24h" on-timeout="reject">
      <notify>
        <slack channels="#approvals #engineering" bot-token="{{secrets.SLACK_BOT_TOKEN}}" app-token="{{secrets.SLACK_APP_TOKEN}}" />
        <slack channels="#management" bot-token="{{secrets.SECOND_SLACK_BOT_TOKEN}}" app-token="{{secrets.SECOND_SLACK_APP_TOKEN}}" />
      </notify>
      <when-approved>
        <step id="approvedRoute"><script>return { route: 'approved' };</script></step>
      </when-approved>
      <when-rejected>
        <step id="rejectedRoute"><script>return { route: 'rejected' };</script></step>
      </when-rejected>
    </approval>
    <step id="finalStatus"><script>return { decision: context.steps.review.decision };</script></step>
  </steps>
</workflow>
</woml>`;
  return compileWoml(parseWoml(source, { file: 'n6-multi-workspace.woml' }));
}

function durableEventTypes(database: string): string[] {
  const sqlite = new Database(database, { readonly: true });
  try {
    return sqlite
      .query('SELECT event_json FROM woml_run_events ORDER BY sequence')
      .all()
      .map(row => JSON.parse((row as { event_json: string }).event_json))
      .map(event => (event as { type: string }).type);
  } finally {
    sqlite.close();
  }
}

describe('Rust and Slack provider journey', () => {
  nativeTest('sends every message, accepts one action, resumes, and updates all messages', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-n4-journey-'));
    const database = join(directory, 'state.sqlite');
    try {
      const compiled = routedWorkflow();
      const waiting = await executeApprovalWorkflowWithRust(compiled, database, {
        nativeCorePath,
        scriptHostPath,
      });
      expect(waiting.status).toBe('waiting');
      if (waiting.status !== 'waiting') throw new Error('expected waiting');

      const journey = await runNotificationProviderJourneyWithRust(
        database,
        waiting.runId,
        {
          nativeCorePath,
          notificationHostPath,
          interactionTimeoutMs: 5_000,
        }
      );
      expect(journey.deliveries).toMatchObject({
        attempted: 2,
        succeeded: 2,
        failed: 0,
      });
      expect(journey.diagnostics).toEqual({
        version: 1,
        deliveryFailures: [],
      });
      expect(journey.decision).toMatchObject({
        status: 'accepted',
        decision: 'approved',
      });
      expect(journey.updates).toMatchObject({
        updatesAttempted: 2,
        updatesSucceeded: 2,
        updatesFailed: 0,
      });

      const resumed = await resumeApprovalWorkflowWithRust(
        compiled,
        database,
        waiting.runId,
        { nativeCorePath, scriptHostPath }
      );
      expect(resumed.status).toBe('succeeded');
      if (resumed.status !== 'succeeded') throw new Error('expected success');
      expect(resumed.execution.result).toEqual({ decision: 'approved' });
      expect(resumed.execution.executionOrder).toContain('approvedRoute');
      expect(resumed.execution.executionOrder).not.toContain('rejectedRoute');
      expect(
        resumed.execution.events.filter(
          event => event.type === 'notification_delivery_succeeded'
        )
      ).toHaveLength(2);
      expect(
        resumed.execution.events.filter(
          event => event.type === 'notification_decision_accepted'
        )
      ).toHaveLength(1);
      expect(
        resumed.execution.events.filter(
          event => event.type === 'notification_message_updated'
        )
      ).toHaveLength(2);

      const serialized = JSON.stringify(resumed);
      expect(serialized).not.toContain('xoxb-n4-secret-bot-value');
      expect(serialized).not.toContain('xapp-n4-secret-app-value');
      expect(serialized).not.toContain('ncap_');
      const databaseBytes = Buffer.from(await Bun.file(database).arrayBuffer());
      expect(databaseBytes.includes(Buffer.from('xoxb-n4-secret-bot-value'))).toBe(
        false
      );
      expect(databaseBytes.includes(Buffer.from('xapp-n4-secret-app-value'))).toBe(
        false
      );
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  nativeTest('converges simultaneous actions from multiple tags and credential sets on one route', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-n6-multi-workspace-'));
    const database = join(directory, 'state.sqlite');
    try {
      const compiled = multiWorkspaceWorkflow();
      const waiting = await executeApprovalWorkflowWithRust(compiled, database, {
        nativeCorePath,
        scriptHostPath,
      });
      if (waiting.status !== 'waiting') throw new Error('expected waiting');

      const journey = await runNotificationProviderJourneyWithRust(
        database,
        waiting.runId,
        {
          nativeCorePath,
          notificationHostPath: multiWorkspaceHostPath,
          interactionTimeoutMs: 5_000,
        }
      );
      expect(journey.deliveries).toMatchObject({
        attempted: 3,
        succeeded: 3,
        failed: 0,
      });
      expect(journey.decision).toMatchObject({
        status: 'accepted',
        decision: 'approved',
      });
      expect(journey.updates).toMatchObject({
        updatesAttempted: 3,
        updatesSucceeded: 3,
        updatesFailed: 0,
      });

      const resumed = await resumeApprovalWorkflowWithRust(
        compiled,
        database,
        waiting.runId,
        { nativeCorePath, scriptHostPath }
      );
      expect(resumed.status).toBe('succeeded');
      if (resumed.status !== 'succeeded') throw new Error('expected success');
      expect(resumed.execution.result).toEqual({ decision: 'approved' });
      expect(
        resumed.execution.events.filter(
          event => event.type === 'notification_decision_accepted'
        )
      ).toHaveLength(1);
      expect(
        resumed.execution.events.filter(
          event => event.type === 'notification_message_updated'
        )
      ).toHaveLength(3);
      expect(
        resumed.execution.executionOrder.filter(id => id === 'approvedRoute')
      ).toHaveLength(1);
      expect(resumed.execution.executionOrder).not.toContain('rejectedRoute');

      const serialized = JSON.stringify({ compiled, journey, resumed });
      for (const secret of [
        'xoxb-n6-primary-bot-value',
        'xapp-n6-primary-app-value',
        'xoxb-n6-secondary-bot-value',
        'xapp-n6-secondary-app-value',
      ]) {
        expect(serialized).not.toContain(secret);
        for (const path of [database, `${database}-wal`, `${database}-shm`]) {
          if (!existsSync(path)) continue;
          const bytes = Buffer.from(await Bun.file(path).arrayBuffer());
          expect(bytes.includes(Buffer.from(secret))).toBe(false);
        }
      }
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  nativeTest('fails explicitly when the provider host cannot resolve credentials', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-n4-missing-secret-'));
    const database = join(directory, 'state.sqlite');
    try {
      const compiled = await workflow();
      const waiting = await executeApprovalWorkflowWithRust(compiled, database, {
        nativeCorePath,
        scriptHostPath,
      });
      if (waiting.status !== 'waiting') throw new Error('expected waiting');
      await expect(
        runNotificationProviderJourneyWithRust(database, waiting.runId, {
          nativeCorePath,
          notificationHostPath: missingSecretHostPath,
          interactionTimeoutMs: 1_000,
        })
      ).rejects.toMatchObject({
        name: NotificationProviderError.name,
        code: 'WOML_NOTIFICATION_DELIVERY_FAILED',
        diagnostics: {
          version: 1,
          deliveryFailures: [
            {
              provider: 'slack',
              destination: '#approvals',
              failure: { code: 'WOML_SECRET_NOT_FOUND' },
            },
            {
              provider: 'slack',
              destination: '#engineering',
              failure: { code: 'WOML_SECRET_NOT_FOUND' },
            },
          ],
        },
      });
      const bytes = Buffer.from(await Bun.file(database).arrayBuffer());
      expect(bytes.includes(Buffer.from('WOML_SECRET_SLACK'))).toBe(false);
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  nativeTest('keeps the approval waiting when no provider action arrives before the deadline', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-n4-timeout-'));
    const database = join(directory, 'state.sqlite');
    try {
      const compiled = await workflow();
      const waiting = await executeApprovalWorkflowWithRust(compiled, database, {
        nativeCorePath,
        scriptHostPath,
      });
      if (waiting.status !== 'waiting') throw new Error('expected waiting');
      await expect(
        runNotificationProviderJourneyWithRust(database, waiting.runId, {
          nativeCorePath,
          notificationHostPath: noActionHostPath,
          interactionTimeoutMs: 25,
        })
      ).rejects.toMatchObject({
        name: NotificationProviderError.name,
        code: 'WOML_NOTIFICATION_INTERACTION_TIMEOUT',
      });
      const resumed = await executeApprovalWorkflowWithRust(compiled, database, {
        nativeCorePath,
        scriptHostPath,
      });
      expect(resumed.status).toBe('waiting');
      const eventTypes = durableEventTypes(database);
      expect(
        eventTypes.filter(type => type === 'notification_delivery_succeeded')
      ).toHaveLength(2);
      expect(eventTypes).not.toContain('notification_decision_accepted');
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  nativeTest('fails closed when the provider host crashes during delivery', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-n4-host-crash-'));
    const database = join(directory, 'state.sqlite');
    try {
      const compiled = await workflow();
      const waiting = await executeApprovalWorkflowWithRust(compiled, database, {
        nativeCorePath,
        scriptHostPath,
      });
      if (waiting.status !== 'waiting') throw new Error('expected waiting');
      await expect(
        runNotificationProviderJourneyWithRust(database, waiting.runId, {
          nativeCorePath,
          notificationHostPath: crashingHostPath,
          interactionTimeoutMs: 1_000,
        })
      ).rejects.toMatchObject({
        name: NotificationProviderError.name,
        code: 'WOML_NOTIFICATION_DELIVERY_FAILED',
      });
      const eventTypes = durableEventTypes(database);
      expect(
        eventTypes.filter(type => type === 'notification_delivery_failed')
      ).toHaveLength(2);
      expect(eventTypes).not.toContain('notification_decision_accepted');
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  nativeTest('uses the durable retry schedule for an explicit Slack rate limit', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-n5-rate-limit-'));
    const database = join(directory, 'state.sqlite');
    try {
      const compiled = await workflow();
      const waiting = await executeApprovalWorkflowWithRust(compiled, database, {
        nativeCorePath,
        scriptHostPath,
      });
      if (waiting.status !== 'waiting') throw new Error('expected waiting');
      const journey = await runNotificationProviderJourneyWithRust(
        database,
        waiting.runId,
        {
          nativeCorePath,
          notificationHostPath: rateLimitedHostPath,
          interactionTimeoutMs: 5_000,
        }
      );
      expect(journey.deliveries).toMatchObject({
        attempted: 4,
        succeeded: 2,
        failed: 2,
        runFailed: false,
      });
      expect(journey.resolution).toBe('approved');
      const eventTypes = durableEventTypes(database);
      expect(
        eventTypes.filter(type => type === 'notification_delivery_failed')
      ).toHaveLength(2);
      expect(
        eventTypes.filter(type => type === 'notification_delivery_succeeded')
      ).toHaveLength(2);
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  nativeTest('settles a retryable sibling even after another provider destination succeeds', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-mixed-delivery-'));
    const database = join(directory, 'state.sqlite');
    try {
      const compiled = routedWorkflow();
      const waiting = await executeApprovalWorkflowWithRust(compiled, database, {
        nativeCorePath,
        scriptHostPath,
      });
      if (waiting.status !== 'waiting') throw new Error('expected waiting');

      const journey = await runNotificationProviderJourneyWithRust(
        database,
        waiting.runId,
        {
          nativeCorePath,
          notificationHostPath: mixedOutcomeHostPath,
          interactionTimeoutMs: 5_000,
        }
      );
      expect(journey.deliveries).toMatchObject({
        attempted: 3,
        succeeded: 2,
        failed: 1,
        runFailed: false,
      });
      expect(journey.updates).toMatchObject({
        updatesAttempted: 2,
        updatesSucceeded: 2,
        updatesFailed: 0,
      });
      expect(journey.resolution).toBe('approved');
      expect(
        durableEventTypes(database).filter(
          type => type === 'notification_delivery_failed'
        )
      ).toHaveLength(1);
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });

  nativeTest('settles a real deadline and updates every Slack message before continuing', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'woml-n5-timeout-update-'));
    const database = join(directory, 'state.sqlite');
    try {
      const compiled = timeoutWorkflow();
      const waiting = await executeApprovalWorkflowWithRust(compiled, database, {
        nativeCorePath,
        scriptHostPath,
      });
      if (waiting.status !== 'waiting') throw new Error('expected waiting');
      const journey = await runNotificationProviderJourneyWithRust(
        database,
        waiting.runId,
        {
          nativeCorePath,
          notificationHostPath: noActionHostPath,
          interactionTimeoutMs: 650,
        }
      );
      expect(journey.decision).toBeNull();
      expect(journey.resolution).toBe('rejected');
      expect(journey.updates).toMatchObject({
        updatesAttempted: 2,
        updatesSucceeded: 2,
        updatesFailed: 0,
      });
      const resumed = await resumeApprovalWorkflowWithRust(
        compiled,
        database,
        waiting.runId,
        { nativeCorePath, scriptHostPath }
      );
      expect(resumed.status).toBe('succeeded');
      if (resumed.status !== 'succeeded') throw new Error('expected success');
      expect(resumed.execution.result).toEqual({ decision: 'rejected' });
      expect(
        resumed.execution.events.filter(
          event => event.type === 'notification_message_updated'
        )
      ).toHaveLength(2);
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  });
});
