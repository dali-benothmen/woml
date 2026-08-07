import { describe, expect, test } from 'bun:test';

import {
  serveApprovalAndWait,
  type ApprovalWaitReason,
} from '../src/approval-server';
import {
  ApprovalDecisionError,
  type ApprovalDecision,
  type ApprovalDecisionResult,
  type RustApprovalRuntimeOutcome,
} from '../src/rust-executor';

type WaitingOutcome = Extract<
  RustApprovalRuntimeOutcome,
  { status: 'waiting' }
>;

const token =
  'apr_0123456789abcdef0123456789abcdef.0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef';

function waitingOutcome(): WaitingOutcome {
  return {
    contract: 'woml.runtime-outcome',
    version: 1,
    status: 'waiting',
    workflowId: 'publish-article',
    runId: 'run_approval_http',
    approval: {
      approvalId: 'editorApproval',
      requestId: 'aprreq_editor_http',
      name: 'Editorial approval',
      description: 'Approve publication',
      onTimeout: 'reject',
      token,
      credentialExpiresAt: '2099-01-01T00:00:00.000Z',
    },
  };
}

function decisionResult(
  decision: ApprovalDecision,
  status: 'accepted' | 'already_resolved' = 'accepted'
): ApprovalDecisionResult {
  return {
    contract: 'woml.approval-http',
    version: 1,
    status,
    runId: 'run_approval_http',
    approvalId: 'editorApproval',
    requestId: 'aprreq_editor_http',
    decision,
    source: 'human',
    decidedAt: '2026-08-07T10:05:00.000Z',
  };
}

async function availablePort(): Promise<number> {
  const probe = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    fetch: () => new Response(),
  });
  const port = probe.port;
  probe.stop(true);
  if (port === undefined) throw new Error('Bun did not assign a test port.');
  return port;
}

async function serverJourney(
  onDecision: (
    token: string,
    decision: ApprovalDecision
  ) => ApprovalDecisionResult
): Promise<{
  readonly url: string;
  readonly wait: Promise<ApprovalWaitReason>;
}> {
  const port = await availablePort();
  let listening!: (url: string) => void;
  const url = new Promise<string>(resolve => {
    listening = resolve;
  });
  const wait = serveApprovalAndWait({
    outcome: waitingOutcome(),
    port,
    onDecision,
    onTimeout: () => ({
      status: 'not_due',
      runId: 'run_approval_http',
      approvalId: 'editorApproval',
      requestId: 'aprreq_editor_http',
      resolution: null,
      settledAt: null,
    }),
    onListening: listening,
  });
  return { url: await url, wait };
}

function post(url: string, body: string, headers?: HeadersInit) {
  const tokenUrl = new URL(url);
  return fetch(
    `${tokenUrl.origin}/api/v1/approvals/${tokenUrl.pathname.slice(
      '/approvals/'.length
    )}/decision`,
    { method: 'POST', headers, body }
  );
}

describe('local approval HTTP v1', () => {
  test('serves a read-only secured page and rejects malformed requests', async () => {
    const journey = await serverJourney((_token, decision) =>
      decisionResult(decision)
    );

    const page = await fetch(journey.url);
    expect(page.status).toBe(200);
    expect(page.headers.get('cache-control')).toBe('no-store');
    expect(page.headers.get('referrer-policy')).toBe('no-referrer');
    expect(page.headers.get('x-frame-options')).toBe('DENY');
    expect(page.headers.get('content-security-policy')).toContain(
      "default-src 'none'"
    );
    const html = await page.text();
    expect(html).toContain('Editorial approval');
    expect(html).not.toContain('context.steps');
    expect(
      (await fetch(`${new URL(journey.url).origin}/approvals/not-a-token`))
        .status
    ).toBe(404);

    expect((await post(journey.url, '{}')).status).toBe(400);
    expect(
      (
        await post(journey.url, '{', {
          'Content-Type': 'application/json',
        })
      ).status
    ).toBe(400);
    expect(
      (
        await post(journey.url, '{"decision":"approved"}', {
          'Content-Type': 'application/json',
          Origin: 'http://example.com',
        })
      ).status
    ).toBe(400);

    const accepted = await post(journey.url, '{"decision":"approved"}', {
      'Content-Type': 'application/json',
    });
    expect(accepted.status).toBe(200);
    expect(await journey.wait).toBe('decision');
  });

  test('returns accepted and idempotent responses for simultaneous duplicates', async () => {
    let calls = 0;
    const journey = await serverJourney((_token, decision) =>
      decisionResult(decision, calls++ === 0 ? 'accepted' : 'already_resolved')
    );
    const responses = await Promise.all([
      post(journey.url, '{"decision":"approved"}', {
        'Content-Type': 'application/json',
      }),
      post(journey.url, '{"decision":"approved"}', {
        'Content-Type': 'application/json',
      }),
    ]);
    expect(responses.map(response => response.status)).toEqual([200, 200]);
    const bodies = await Promise.all(
      responses.map(response => response.json())
    );
    expect(bodies.map(body => body.status).sort()).toEqual([
      'accepted',
      'already_resolved',
    ]);
    await journey.wait;
  });

  test('maps conflicting and expired Rust decisions without leaking tokens', async () => {
    for (const [code, expectedStatus] of [
      ['WOML_APPROVAL_DECISION_CONFLICT', 409],
      ['WOML_APPROVAL_TOKEN_EXPIRED', 410],
      ['WOML_APPROVAL_EXPIRED', 410],
    ] as const) {
      const journey = await serverJourney(() => {
        throw new ApprovalDecisionError(code, 'sensitive internal detail');
      });
      const response = await post(journey.url, '{"decision":"rejected"}', {
        'Content-Type': 'application/json',
      });
      expect(response.status).toBe(expectedStatus);
      const body = await response.text();
      expect(body).toContain(code);
      expect(body).not.toContain(token);
      expect(await journey.wait).toBe('refresh');
    }
  });

  test('rotates an expiring capability without resolving the approval', async () => {
    const port = await availablePort();
    const outcome = waitingOutcome();
    const wait = serveApprovalAndWait({
      outcome: {
        ...outcome,
        approval: {
          ...outcome.approval,
          credentialExpiresAt: new Date(Date.now() + 30).toISOString(),
        },
      },
      port,
      onDecision: (_token, decision) => decisionResult(decision),
      onTimeout: () => {
        throw new Error('Credential refresh must not settle the approval.');
      },
      onListening: () => {},
    });
    expect(await wait).toBe('refresh');
  });
});
