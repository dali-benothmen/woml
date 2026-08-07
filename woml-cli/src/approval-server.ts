import { randomBytes } from 'node:crypto';

import {
  ApprovalDecisionError,
  type ApprovalDecision,
  type ApprovalDecisionResult,
  type ApprovalTimeoutResult,
  type RustApprovalRuntimeOutcome,
} from './rust-executor';

export const DEFAULT_APPROVAL_PORT = 7331;

type WaitingOutcome = Extract<
  RustApprovalRuntimeOutcome,
  { status: 'waiting' }
>;

export type ApprovalWaitReason = 'decision' | 'timeout' | 'refresh';

export interface ApprovalServerOptions {
  readonly outcome: WaitingOutcome;
  readonly port: number;
  readonly onDecision: (
    token: string,
    decision: ApprovalDecision
  ) => ApprovalDecisionResult;
  readonly onTimeout: (
    runId: string,
    approvalId: string
  ) => ApprovalTimeoutResult;
  readonly onListening: (url: string) => void;
}

export class ApprovalServerBindError extends Error {
  readonly code = 'WOML_APPROVAL_SERVER_BIND_FAILED';

  constructor(port: number) {
    super(`Could not bind the local approval server to 127.0.0.1:${port}.`);
    this.name = 'ApprovalServerBindError';
  }
}

function securityHeaders(contentType: string): HeadersInit {
  return {
    'Cache-Control': 'no-store',
    'Content-Type': contentType,
    'Referrer-Policy': 'no-referrer',
    'X-Content-Type-Options': 'nosniff',
    'X-Frame-Options': 'DENY',
  };
}

function json(value: unknown, status: number): Response {
  return Response.json(value, {
    status,
    headers: securityHeaders('application/json; charset=utf-8'),
  });
}

function approvalError(
  status: number,
  code:
    | 'WOML_APPROVAL_REQUEST_INVALID'
    | 'WOML_APPROVAL_TOKEN_INVALID'
    | 'WOML_APPROVAL_TOKEN_EXPIRED'
    | 'WOML_APPROVAL_EXPIRED'
    | 'WOML_APPROVAL_DECISION_CONFLICT'
    | 'WOML_APPROVAL_INTERNAL',
  message: string
): Response {
  return json(
    {
      contract: 'woml.approval-http',
      version: 1,
      error: { code, message },
    },
    status
  );
}

function invalidRequest(message = 'Decision must be approved or rejected.') {
  return approvalError(400, 'WOML_APPROVAL_REQUEST_INVALID', message);
}

function errorResponse(error: unknown): {
  readonly response: Response;
  readonly shouldResume: boolean;
} {
  if (!(error instanceof ApprovalDecisionError)) {
    return {
      response: approvalError(
        500,
        'WOML_APPROVAL_INTERNAL',
        'The approval decision could not be safely confirmed.'
      ),
      shouldResume: false,
    };
  }
  switch (error.code) {
    case 'WOML_APPROVAL_TOKEN_INVALID':
      return {
        response: approvalError(
          404,
          error.code,
          'The approval capability is invalid.'
        ),
        shouldResume: false,
      };
    case 'WOML_APPROVAL_TOKEN_EXPIRED':
      return {
        response: approvalError(
          410,
          error.code,
          'The approval capability expired.'
        ),
        shouldResume: true,
      };
    case 'WOML_APPROVAL_EXPIRED':
      return {
        response: approvalError(
          410,
          error.code,
          'The approval request expired.'
        ),
        shouldResume: true,
      };
    case 'WOML_APPROVAL_DECISION_CONFLICT':
      return {
        response: approvalError(
          409,
          error.code,
          'A different human decision is already durable.'
        ),
        shouldResume: true,
      };
    case 'WOML_APPROVAL_INTERNAL':
      return {
        response: approvalError(
          500,
          error.code,
          'The approval decision could not be safely confirmed.'
        ),
        shouldResume: false,
      };
  }
}

function escapeHtml(value: string): string {
  return value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;');
}

function approvalPage(outcome: WaitingOutcome, nonce: string): string {
  const { approval } = outcome;
  const title = approval.name ?? `Approval ${approval.approvalId}`;
  const description = approval.description ?? 'This workflow needs a decision.';
  const deadline =
    approval.expiresAt === undefined
      ? 'No workflow deadline'
      : new Date(approval.expiresAt).toLocaleString();
  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>${escapeHtml(title)}</title>
  <style nonce="${nonce}">
    :root { color-scheme: light dark; font-family: system-ui, sans-serif; }
    body { margin: 0; min-height: 100vh; display: grid; place-items: center; background: Canvas; color: CanvasText; }
    main { width: min(38rem, calc(100% - 3rem)); padding: 2rem; border: 1px solid color-mix(in srgb, CanvasText 20%, transparent); border-radius: 1rem; }
    h1 { margin-top: 0; } .meta { opacity: .7; } .actions { display: flex; gap: .75rem; margin-top: 1.5rem; }
    button { border: 0; border-radius: .6rem; padding: .75rem 1rem; font: inherit; cursor: pointer; }
    button[data-decision="approved"] { background: #16803b; color: white; }
    button[data-decision="rejected"] { background: #b42318; color: white; }
    #status { min-height: 1.5rem; margin-bottom: 0; }
  </style>
</head>
<body>
  <main>
    <p class="meta">Workflow: ${escapeHtml(outcome.workflowId)}</p>
    <h1>${escapeHtml(title)}</h1>
    <p>${escapeHtml(description)}</p>
    <p class="meta">Deadline: ${escapeHtml(deadline)}</p>
    <div class="actions">
      <button type="button" data-decision="approved">Approve</button>
      <button type="button" data-decision="rejected">Reject</button>
    </div>
    <p id="status" role="status" aria-live="polite"></p>
  </main>
  <script nonce="${nonce}">
    const status = document.querySelector('#status');
    for (const button of document.querySelectorAll('button[data-decision]')) {
      button.addEventListener('click', async () => {
        for (const item of document.querySelectorAll('button')) item.disabled = true;
        status.textContent = 'Recording decision…';
        const token = location.pathname.slice('/approvals/'.length);
        try {
          const response = await fetch('/api/v1/approvals/' + token + '/decision', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ decision: button.dataset.decision })
          });
          const body = await response.json();
          status.textContent = response.ok
            ? 'Decision recorded. You may close this page.'
            : body.error?.message ?? 'The decision could not be recorded.';
          if (!response.ok) for (const item of document.querySelectorAll('button')) item.disabled = false;
        } catch {
          status.textContent = 'The local WOML process is no longer available.';
        }
      });
    }
  </script>
</body>
</html>`;
}

function scheduleAt(deadline: string, callback: () => void): () => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let cancelled = false;
  const schedule = () => {
    if (cancelled) return;
    const remaining = Date.parse(deadline) - Date.now();
    if (remaining <= 0) {
      callback();
      return;
    }
    timer = setTimeout(schedule, Math.min(remaining, 0x7fff_ffff));
  };
  schedule();
  return () => {
    cancelled = true;
    if (timer !== undefined) clearTimeout(timer);
  };
}

export async function serveApprovalAndWait(
  options: ApprovalServerOptions
): Promise<ApprovalWaitReason> {
  const { outcome } = options;
  const tokenPath = `/approvals/${outcome.approval.token}`;
  const decisionPath = `/api/v1/approvals/${outcome.approval.token}/decision`;
  const origin = `http://127.0.0.1:${options.port}`;
  let complete!: (reason: ApprovalWaitReason) => void;
  let completed = false;
  const waiting = new Promise<ApprovalWaitReason>(resolve => {
    complete = reason => {
      if (completed) return;
      completed = true;
      resolve(reason);
    };
  });
  const finishAfterResponse = (reason: ApprovalWaitReason) => {
    setTimeout(() => complete(reason), 0);
  };

  let server: Bun.Server<undefined>;
  try {
    server = Bun.serve({
      hostname: '127.0.0.1',
      port: options.port,
      async fetch(request) {
        const url = new URL(request.url);
        if (request.method === 'GET' && url.pathname === tokenPath) {
          const nonce = randomBytes(18).toString('base64url');
          return new Response(approvalPage(outcome, nonce), {
            status: 200,
            headers: {
              ...securityHeaders('text/html; charset=utf-8'),
              'Content-Security-Policy': `default-src 'none'; script-src 'nonce-${nonce}'; style-src 'nonce-${nonce}'; connect-src 'self'; img-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'`,
            },
          });
        }
        if (request.method !== 'POST' || url.pathname !== decisionPath) {
          return approvalError(
            404,
            'WOML_APPROVAL_TOKEN_INVALID',
            'The approval capability is invalid.'
          );
        }
        const requestOrigin = request.headers.get('origin');
        if (requestOrigin !== null && requestOrigin !== origin) {
          return invalidRequest('The request origin is not allowed.');
        }
        const contentType = request.headers.get('content-type');
        if (
          contentType?.split(';', 1)[0]?.trim().toLowerCase() !==
          'application/json'
        ) {
          return invalidRequest('Content-Type must be application/json.');
        }
        const text = await request.text();
        if (new TextEncoder().encode(text).byteLength > 4096) {
          return invalidRequest('The approval request body is too large.');
        }
        let body: unknown;
        try {
          body = JSON.parse(text);
        } catch {
          return invalidRequest(
            'The approval request body must be valid JSON.'
          );
        }
        if (
          typeof body !== 'object' ||
          body === null ||
          Array.isArray(body) ||
          Object.keys(body).length !== 1 ||
          !('decision' in body) ||
          (body.decision !== 'approved' && body.decision !== 'rejected')
        ) {
          return invalidRequest();
        }
        try {
          const result = options.onDecision(
            outcome.approval.token,
            body.decision
          );
          finishAfterResponse('decision');
          return json(result, 200);
        } catch (error) {
          if (
            error instanceof ApprovalDecisionError &&
            error.code === 'WOML_APPROVAL_EXPIRED'
          ) {
            try {
              options.onTimeout(outcome.runId, outcome.approval.approvalId);
            } catch {
              // The frozen HTTP response remains expiry-safe; continuation will
              // surface any durable runtime failure after this response.
            }
          }
          const mapped = errorResponse(error);
          if (mapped.shouldResume) finishAfterResponse('refresh');
          return mapped.response;
        }
      },
    });
  } catch {
    throw new ApprovalServerBindError(options.port);
  }

  const stopDeadline =
    outcome.approval.expiresAt === undefined
      ? () => {}
      : scheduleAt(outcome.approval.expiresAt, () => {
          try {
            const settlement = options.onTimeout(
              outcome.runId,
              outcome.approval.approvalId
            );
            if (settlement.status === 'not_due') return;
            complete('timeout');
          } catch {
            complete('timeout');
          }
        });
  const credentialExpiresBeforeDeadline =
    outcome.approval.expiresAt === undefined ||
    Date.parse(outcome.approval.credentialExpiresAt) <
      Date.parse(outcome.approval.expiresAt);
  const stopCredentialRefresh = credentialExpiresBeforeDeadline
    ? scheduleAt(outcome.approval.credentialExpiresAt, () =>
        complete('refresh')
      )
    : () => {};

  options.onListening(`${origin}${tokenPath}`);
  try {
    return await waiting;
  } finally {
    stopDeadline();
    stopCredentialRefresh();
    server.stop(true);
  }
}
