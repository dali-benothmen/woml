import { describe, expect, test } from 'bun:test';
import { analyzeHistoryWork, reviewedHistorySha256 } from '../scripts/analyze-history-replay';

describe('history replay investigation cost model (not validation)', () => {
  test('names a reviewed source fingerprint without freezing future engine edits', () => {
    // The standalone analyzer checks this fingerprint at execution time. The
    // repository test must not prohibit a later reviewed optimization.
    expect(reviewedHistorySha256).toMatch(/^[a-f0-9]{64}$/u);
  });

  test('counts the policy-managed sequential-32 history without asserting a speedup', () => {
    const events = [{ type: 'run_admitted' }, { type: 'run_execution_started' }];
    for (let step = 0; step < 32; step++) events.push({ type: 'step_attempt_started' }, { type: 'step_attempt_succeeded' });
    events.push({ type: 'run_outcome_decided' }, { type: 'run_finalized' });
    expect(analyzeHistoryWork(events)).toEqual({ inputEvents: 68, prefixRequests: 33,
      currentFullFoldCalls: 34, currentEventVisits: 1190, replayedPrefixEventVisits: 1122,
      proposedEventVisits: 134, cursorEventVisits: 66 });
  });

  test('accounts for inclusive legacy success and already-consumed cursor positions', () => {
    expect(analyzeHistoryWork([{ type: 'run_started' }, { type: 'run_succeeded' }]))
      .toEqual({ inputEvents: 2, prefixRequests: 1, currentFullFoldCalls: 2,
        currentEventVisits: 4, replayedPrefixEventVisits: 2, proposedEventVisits: 4, cursorEventVisits: 2 });
    expect(analyzeHistoryWork([]).proposedEventVisits).toBe(0);
  });

  test('skips scoped step checks but includes their events in later global prefixes', () => {
    const result = analyzeHistoryWork([
      { type: 'run_started' }, { type: 'step_attempt_started', iteration: { index: 0 } },
      { type: 'step_attempt_succeeded', iteration: { index: 0 } },
      { type: 'lifecycle_hook_completed' }, { type: 'run_outcome_decided' },
    ]);
    expect(result.prefixRequests).toBe(2);
    expect(result.replayedPrefixEventVisits).toBe(7);
    expect(result.cursorEventVisits).toBe(4);
  });

  test('models success-only fork checks without counting failed branches', () => {
    const result = analyzeHistoryWork([
      { type: 'run_started' }, { type: 'fork_branch_settled', data: { outcome: 'failed' } },
      { type: 'fork_branch_settled', data: { outcome: 'succeeded' } },
    ]);
    expect(result.prefixRequests).toBe(1);
    expect(result.replayedPrefixEventVisits).toBe(2);
  });
});
