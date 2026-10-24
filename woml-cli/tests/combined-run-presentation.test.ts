import { describe, expect, test } from 'bun:test';

import {
  createCombinedRunPresentation,
  createProjectActivationPresentation,
  renderCombinedRunPresentation,
  renderProjectActivationPresentation,
} from '../src/combined-run-presentation';
import type {
  RustArtifactInspectionV1,
  RustRunInspectionV2,
} from '../src/rust-executor';

function inspection(
  overrides: Partial<RustRunInspectionV2> = {}
): RustRunInspectionV2 {
  return {
    profile: 'woml.run-inspection/v11',
    runId: 'run_rehearsal_42',
    workflowId: 'infrastructure-agent-deployment',
    status: 'running',
    businessOutcome: 'undecided',
    lifecycleStatus: 'running',
    hooks: [],
    warnings: [],
    cancellation: { requested: false },
    ...overrides,
  };
}

describe('Combined developer experience presentation', () => {
  test('renders one concise project activation summary', () => {
    const value = createProjectActivationPresentation({
      project: 'infrastructure-agent',
      digest: `sha256:${'a'.repeat(64)}`,
      workflows: ['infrastructure-agent-deployment'],
      statePath: '/tmp/rehearsal/state.sqlite',
      policy: 'infrastructure-agent-rehearsal@1',
      targets: ['production-private'],
      openTelemetry: true,
    });
    const human = renderProjectActivationPresentation(value);
    expect(human).toContain('PROJECT  infrastructure-agent · aaaaaaaaaaaa');
    expect(human).toContain('Workflow  infrastructure-agent-deployment');
    expect(human).toContain('Target    production-private');
    expect(human).toContain('Status    Ready');
    expect(JSON.parse(renderProjectActivationPresentation(value, true))).toEqual(
      value
    );
  });

  test('makes approval and signal blocking states explicit', () => {
    const approval = createCombinedRunPresentation({
      inspection: inspection({
        approvals: {
          pending: 1,
          truncated: false,
          items: [
            {
              approvalId: 'approveDeployment',
              requestId: 'approval_42',
              status: 'pending',
              reviewDigest: `sha256:${'b'.repeat(64)}`,
              review: {},
              required: 2,
              received: 1,
              requiredRoles: ['production-owner', 'security-owner'],
              representedRoles: ['production-owner'],
              directoryVersion: 'rehearsal-1',
              requestedAt: '2026-10-25T10:00:00.000Z',
              decisions: [],
              totalDecisions: 0,
              decisionsTruncated: false,
            },
          ],
        },
      }),
      statePath: '/tmp/state.sqlite',
    });
    expect(approval).toMatchObject({
      state: 'waiting-for-approval',
      detail: 'approveDeployment · 1/2 approvals',
    });

    const signal = createCombinedRunPresentation({
      inspection: inspection(),
      statePath: '/tmp/state.sqlite',
      waits: {
        profile: 'woml.wait-inspection/v1',
        runId: 'run_rehearsal_42',
        pendingWaits: 1,
        waits: [
          {
            runId: 'run_rehearsal_42',
            waitId: 'rolloutSignal',
            name: 'Wait for rollout',
            kind: 'signal',
            status: 'pending',
            registeredAt: '2026-10-25T10:00:00.000Z',
            signals: ['rollout.succeeded', 'rollout.failed'],
          },
        ],
      },
    });
    expect(signal.state).toBe('waiting-for-signal');
    expect(signal.nextCommands.join('\n')).toContain(
      'woml signal "rollout.succeeded"'
    );
    expect(renderCombinedRunPresentation(signal)).toContain(
      'Current: waiting for signal'
    );
  });

  test('prioritizes cancellation, correction, and ambiguous remote outcomes', () => {
    expect(
      createCombinedRunPresentation({
        inspection: inspection({ cancellation: { requested: true } }),
        statePath: '/tmp/state.sqlite',
      }).state
    ).toBe('cancellation-requested');
    expect(
      createCombinedRunPresentation({
        inspection: inspection(),
        statePath: '/tmp/state.sqlite',
        correction: {
          profile: 'woml.correction-inspection/v1',
          runId: 'run_rehearsal_42',
          phase: 'compensating',
          originalOutcome: 'failed',
          sagas: [],
          unresolvedActions: [],
          truncated: false,
        },
      }).state
    ).toBe('compensating');
    const ambiguous = createCombinedRunPresentation({
      inspection: inspection(),
      statePath: '/tmp/state.sqlite',
      remoteDispatch: {
        profile: 'woml.remote-dispatch-inspection/v1',
        dispatchId: 'dispatch_42',
        runId: 'run_rehearsal_42',
        nodeId: 'applyPlan',
        attempt: 1,
        target: 'production-private',
        state: 'lost',
        ambiguous: true,
        outputSequence: 7,
      },
    });
    expect(ambiguous.state).toBe('ambiguous-remote-outcome');
    expect(ambiguous.nextCommands.join('\n')).toContain(
      'woml dispatch reconcile "dispatch_42"'
    );
  });

  test('keeps commands bounded, redacted, and JSON stable', () => {
    const artifact = {
      artifact: {
        contract: 'woml.artifact-reference',
        contractVersion: 1,
        id: 'artifact_42',
        name: 'plan.json',
        size: 42,
        sha256: 'a'.repeat(64),
        contentType: 'application/json',
      },
    } as unknown as RustArtifactInspectionV1;
    const value = createCombinedRunPresentation({
      inspection: inspection({ status: 'succeeded' }),
      statePath: '/tmp/state\u001b[31m.sqlite',
      artifacts: [artifact],
    });
    expect(value.state).toBe('completed');
    expect(value.nextCommands.length).toBeLessThanOrEqual(8);
    expect(value.nextCommands.join('\n')).not.toContain('\u001b[');
    expect(JSON.parse(renderCombinedRunPresentation(value, true))).toEqual(value);
  });
});
