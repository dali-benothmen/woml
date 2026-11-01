#!/usr/bin/env bun
// Read-only cost model for the reviewed validator, NOT a workflow validator or
// an implemented optimization. Event visits are algorithmic counts, not timing.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';

export const reviewedHistorySha256 = 'ea0b7221964cebf4fd0be40ba9e506f93868304ba5e4c12cddcbf31a231c5536';

interface WorkEvent {
  readonly type: string;
  readonly iteration?: unknown;
  readonly data?: { readonly outcome?: unknown };
}

// engine/history.rs rebuilds the exclusive prefix in each of these arms.
const exclusivePrefixTypes = new Set([
  'choice_selected', 'fork_opened', 'fork_join_settled', 'saga_started',
  'correction_attempt_started', 'saga_correction_completed', 'run_outcome_decided',
  'parallel_group_started', 'step_attempt_started', 'approval_requested',
  'approval_review_requested', 'wait_registered', 'lifecycle_action_attempt_started',
  'lifecycle_hook_completed',
]);

export function analyzeHistoryWork(events: readonly WorkEvent[]) {
  let prefixRequests = 0, replayedPrefixEventVisits = 0, cursorEventVisits = 0;
  for (const [index, event] of events.entries()) {
    // Scoped ordinary events take the validator's early-continue path. Loop
    // boundary events have no prefix request in their own match arms either.
    if (event.iteration !== undefined) continue;
    const exclusive = exclusivePrefixTypes.has(event.type)
      || (event.type === 'fork_branch_settled' && event.data?.outcome === 'succeeded');
    const length = exclusive ? index : event.type === 'run_succeeded' ? index + 1 : undefined;
    if (length === undefined) continue;
    prefixRequests += 1;
    replayedPrefixEventVisits += length;
    cursorEventVisits = Math.max(cursorEventVisits, length);
  }
  return {
    inputEvents: events.length,
    prefixRequests,
    currentFullFoldCalls: 1 + prefixRequests,
    currentEventVisits: events.length + replayedPrefixEventVisits,
    replayedPrefixEventVisits,
    // Keep the initial full-history structural gate. A local monotone cursor
    // would subsequently apply each required prefix event at most once.
    proposedEventVisits: events.length + cursorEventVisits,
    cursorEventVisits,
  };
}

async function main() {
  const args = process.argv.slice(2);
  assert.ok(args.length === 1 || (args.length === 3 && args[1] === '--output'),
    'Usage: bun scripts/analyze-history-replay.ts measurements.json [--output NEW.json]');
  const repository = resolve(import.meta.dir, '../..');
  const measurementsPath = resolve(args[0]!);
  const measurements = JSON.parse(await readFile(measurementsPath, 'utf8'));
  assert.equal(measurements.manifest.profile, 'woml.runtime-investigation/v1');
  assert.equal(measurements.manifest.comparison, undefined, 'This is a baseline cost model, not a candidate work counter');
  assert.match(measurements.manifest.sourceCommit, /^[a-f0-9]{40}$/u);
  const original = Bun.spawnSync(['git', 'show', `${measurements.manifest.sourceCommit}:core/woml-engine/src/engine/history.rs`],
    { cwd: repository, stdout: 'pipe', stderr: 'pipe' });
  assert.equal(original.exitCode, 0, 'The recorded baseline source commit must be available locally');
  assert.equal(createHash('sha256').update(original.stdout).digest('hex'), reviewedHistorySha256,
    'Baseline validator differs from the reviewed cost model');
  const { Database } = await import('bun:sqlite');
  const histories = [];
  for (const observation of measurements.observations) {
    for (const [index, sample] of observation.result.samples.entries()) {
      const statePath = resolve(dirname(measurementsPath), observation.directory, `state-${index}.sqlite`);
      const database = new Database(statePath, { readonly: true });
      try {
        const rows = database.query('SELECT event_json FROM woml_run_events WHERE run_id = ?1 ORDER BY sequence')
          .all(sample.runId) as { event_json: string }[];
        const events = rows.map(row => JSON.parse(row.event_json) as WorkEvent);
        assert.equal(events.length, sample.eventCount, 'Stored history no longer matches the recorded experiment');
        histories.push({ case: observation.case, build: observation.build, batch: observation.batch,
          temperature: sample.temperature, runId: sample.runId, work: analyzeHistoryWork(events) });
      } finally { database.close(); }
    }
  }
  const result = { profile: 'woml.history-replay-analysis/v1', sourceCommit: measurements.manifest.sourceCommit,
    reviewedHistorySha256, measurementsPath,
    scope: 'One final history validation per stored run; not all validations during execution. Proposed visits are a cost model, not implemented behavior or measured speedup.',
    histories };
  const json = `${JSON.stringify(result, null, 2)}\n`;
  if (args[2]) {
    // Preserve the original measurements and refuse to overwrite other output.
    await writeFile(resolve(args[2]), json, { flag: 'wx' });
    console.log(`Analyzed ${histories.length} histories; wrote ${resolve(args[2])}`);
  } else process.stdout.write(json);
}

if (import.meta.main) await main();
