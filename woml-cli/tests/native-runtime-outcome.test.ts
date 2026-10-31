import { expect, test } from 'bun:test';
import { parseApprovalRuntimeOutcome } from '../src/rust-executor';

function completed(event: Record<string, unknown>) {
  return JSON.stringify({
    contract: 'woml.runtime-outcome', version: 1, status: 'succeeded',
    execution: {
      workflowId: 'lab', runId: 'run_lab', terminalNodeId: 'done', result: true,
      context: { trigger: {}, steps: {} }, executionOrder: ['done'],
      events: [{ eventSchemaVersion: 20, eventId: 'evt_lab', runId: 'run_lab', sequence: 1,
        occurredAt: '2026-10-31T00:00:00Z', type: 'step_attempt_succeeded', data: {}, ...event }],
    },
  });
}

test('native outcomes accept historical and current events including bounded loop scope', () => {
  for (let eventSchemaVersion = 1; eventSchemaVersion <= 20; eventSchemaVersion++) {
    expect(parseApprovalRuntimeOutcome(completed({ eventSchemaVersion })).status).toBe('succeeded');
  }
  expect(parseApprovalRuntimeOutcome(completed({ iteration: { forEachId: 'zones', index: 2 } })).status).toBe('succeeded');
});

test('native outcomes reject future versions and malformed or widened loop scope', () => {
  for (const event of [
    { eventSchemaVersion: 21 }, { eventSchemaVersion: 0 },
    { eventSchemaVersion: 14, iteration: { forEachId: 'zones', index: 0 } },
    { iteration: { forEachId: 'zones', index: -1 } },
    { iteration: { forEachId: 'zones', index: 10000 } },
    { iteration: { forEachId: 'zones', index: 0.5 } },
    { iteration: { forEachId: '../escape', index: 0 } },
    { iteration: { forEachId: 'x'.repeat(257), index: 0 } },
    { iteration: { forEachId: 'zones', index: 0, payload: 'unreviewed' } },
    { unexpected: true },
  ]) expect(() => parseApprovalRuntimeOutcome(completed(event))).toThrow('invalid runtime outcome');
});
