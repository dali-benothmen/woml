import type { InspectorRenderState, InspectorRunV1, InspectorSnapshotV1 } from '../runtime-inspector';
import { elapsed, fit, frame, paint, safeText, statusTone, uptime, type Tone } from './terminal';

export const INSPECTOR_VIEWS = ['overview', 'runs', 'triggers', 'approvals', 'queues', 'failures', 'runtime'] as const;
const title = (value: string): string => value[0]!.toUpperCase() + value.slice(1);
const cell = (value: string, width: number, tone: Tone, color: boolean, right = false): string =>
  paint(fit(value, width, right ? 'right' : 'left'), tone, color);

export function visibleInspectorRuns(snapshot: InspectorSnapshotV1, state: InspectorRenderState): readonly InspectorRunV1[] {
  const query = state.filter.toLowerCase();
  return snapshot.runs.filter(run =>
    (state.workflowId === undefined || run.workflowId === state.workflowId) &&
    (state.view !== 'approvals' || run.status === 'waiting') &&
    (state.view !== 'queues' || ['queued', 'retrying'].includes(run.status)) &&
    (state.view !== 'failures' || run.status === 'failed') &&
    (!query || `${run.workflowId} ${run.runId} ${run.status}`.toLowerCase().includes(query))
  );
}

export function inspectorMessage(state: InspectorRenderState): string {
  if (state.confirmationRunId !== undefined) return `Cancel ${state.confirmationRunId}? Press y to confirm or n to keep it running.`;
  if (state.searchInput !== undefined) return `Search: ${state.searchInput}█  (Enter apply, Esc cancel)`;
  if (state.analyticsWorkflowId !== undefined && !state.stale) return state.connectionMessage ?? 'PgUp/PgDn scroll   ← back   R refresh   ? help   Q quit';
  return state.connectionMessage ?? (state.stale ? 'STALE · reconnecting; displayed data may be out of date.' :
    state.filter ? `Filter: ${state.filter}` : 'Tab / V views   Enter details   c cancel   R refresh   ? help');
}

export function inspectorFooter(width: number, color: boolean, analytics: boolean): string {
  const hints = analytics
    ? [['↑', 'Scroll Up'], ['↓', 'Scroll Down'], ['→', 'Analytics'], ['←', 'Back To List'], ['/', 'Search'], ['Q', 'Quit']]
    : [['↑', 'Previous'], ['↓', 'Next'], ['→', 'Analytics'], ['←', 'Back To List'], ['/', 'Search'], ['Q', 'Quit']];
  if (width < 104) return fit(analytics ? '↑↓ Scroll  ← Back To List  Q Quit' : '↑↓ Select  → Analytics  / Search  Q Quit', width);
  let result = '', used = 0;
  for (const [key, label] of hints) {
    const text = ` ${key} ${label} `;
    const disabled = analytics ? key === '→' || key === '/' : key === '←';
    result += paint(text, disabled ? 'muted' : 'button', color) + ' ';
    used += Bun.stringWidth(text) + 1;
  }
  return result + ' '.repeat(Math.max(0, width - used));
}

export function renderRunsDashboard(snapshot: InspectorSnapshotV1, state: InspectorRenderState, columns: number, rows: number, color: boolean): string {
  const width = Math.max(1, Math.floor(columns)), height = Math.max(1, Math.floor(rows));
  const runs = visibleInspectorRuns(snapshot, state);
  const all = snapshot.runs.filter(run => state.workflowId === undefined || run.workflowId === state.workflowId);
  const counts = [['running', 'retrying'], ['queued'], ['waiting'], ['failed']]
    .map(statuses => all.filter(run => statuses.includes(run.status)).length);
  const health = snapshot.ready ? 'healthy' : snapshot.lifecycle;
  const workflows = snapshot.workflows.filter(workflow => state.workflowId === undefined || workflow.workflowId === state.workflowId);
  const lines: string[] = [''];
  if (width >= 82 && height >= 16) {
    const half = Math.floor(width * 0.51), right = width - half;
    const labels = ['Active', 'Queue', 'Waits', 'Failed'];
    const tones: Tone[] = ['green', 'blue', 'yellow', 'red'];
    const info = [
      cell(' WOML INSPECT', 15, 'green', color) + cell(`runtime: ${health}${state.stale ? ' · STALE' : ''}`, right - 15, state.stale ? 'yellow' : statusTone(health), color),
      cell(` Workflows: ${workflows.length}   Triggers: ${workflows.reduce((sum, workflow) => sum + workflow.triggerTypes.length, 0)}`, right, 'muted', color),
      cell(` Runs: ${all.length}   ${counts[0]} active, ${counts[3]} failed`, right, 'white', color),
      cell(` Uptime: ${uptime(snapshot.uptimeMs)}   View `, 30, 'muted', color) + cell(`[ ${title(state.view)} ▾ ]`, right - 30, 'white', color),
    ];
    for (let index = 0; index < 4; index++) {
      const meterWidth = Math.min(22, half - 22);
      const filled = all.length === 0 ? 0 : Math.max(Number(counts[index]! > 0), Math.round(counts[index]! / all.length * meterWidth));
      const meter = cell(`  ${labels[index]}`, 9, 'muted', color) + '[' + paint('|'.repeat(filled), tones[index]!, color) + paint((color ? '|' : '·').repeat(meterWidth - filled), 'muted', color) + ']';
      lines.push(meter + cell(`${counts[index]}/${all.length}`, half - meterWidth - 11, tones[index]!, color, true) + info[index]);
    }
    lines.push('');
  } else {
    lines.push(cell(` WOML INSPECT  ${health}${state.stale ? ' · STALE' : ''}`, width, state.stale ? 'yellow' : 'green', color));
    lines.push(fit(` Runs ${all.length}  Active ${counts[0]}  Queue ${counts[1]}  Waits ${counts[2]}  Failed ${counts[3]}`, width));
  }
  const wide = width >= 82;
  const statusWidth = wide ? 14 : 11;
  const durationWidth = wide ? 10 : 8;
  const workflowWidth = wide ? Math.floor((width - statusWidth - durationWidth - 1) * 0.39) : Math.max(0, Math.floor((width - statusWidth - durationWidth - 1) * 0.55));
  const idWidth = wide ? Math.floor((width - statusWidth - durationWidth - workflowWidth - 1) * 0.38) : Math.max(0, width - statusWidth - durationWidth - workflowWidth - 1);
  const workWidth = wide ? width - statusWidth - durationWidth - workflowWidth - idWidth - 1 : 0;
  const tableRow = (marker: string, status: string, workflow: string, id: string, work: string, duration: string): string =>
    fit(marker + status, statusWidth) + fit(workflow, workflowWidth) + fit(id, idWidth) + fit(work, workWidth) + fit(duration, durationWidth);
  lines.push(paint(tableRow(' ', 'STATUS', 'WORKFLOW', 'RUN ID', 'WORK', 'DURATION') + ' ', 'header', color));
  const selected = Math.max(0, Math.min(state.selected, runs.length - 1));
  const chosen = runs[selected];
  const details = chosen === undefined ? ['Selected: —', 'Execution: —'] : [
    `Selected: ${chosen.runId}`,
    `Execution: ${chosen.workflowId} → ${chosen.currentNodeId ?? chosen.status} [${chosen.status}]`,
    ...(state.expanded ? [
      ...(chosen.parentRunId === undefined ? [] : [`Called by: ${chosen.parentRunId}`]),
      ...(chosen.forEach ?? []).slice(0, 3).map(loop => `For each ${loop.forEachId}: ${loop.succeeded + loop.failed + loop.skipped}/${loop.total} completed · ${loop.active} active · ${loop.pending} pending · concurrency ${loop.concurrency}`),
    ] : []),
  ];
  const count = Math.max(1, height - lines.length - details.length - 3);
  const first = Math.max(0, selected - count + 1);
  const shown = runs.slice(first, first + count);
  const thumb = runs.length > count ? Math.round(selected / Math.max(1, runs.length - 1) * (count - 1)) : -1;
  for (let index = 0; index < count; index++) {
    const run = shown[index];
    if (run === undefined) {
      lines.push(index === 0 ? cell(state.filter ? ' No runs match the current filter.' : ' No runs in this view.', width, 'muted', color) : '');
      continue;
    }
    const loop = run.forEach?.find(item => item.status === 'running') ?? run.forEach?.at(-1);
    const work = loop === undefined ? run.currentNodeId ?? '—' : `${loop.forEachId} ${loop.succeeded + loop.failed + loop.skipped}/${loop.total}`;
    const raw = tableRow(first + index === selected ? '›' : ' ', run.status, run.workflowId, run.runId, work, elapsed(run.durationMs));
    const scrollbar = thumb < 0 ? ' ' : index === thumb ? '┃' : '│';
    if (first + index === selected) lines.push(paint(raw + scrollbar, 'selected', color));
    else lines.push(cell(' ' + run.status, statusWidth, statusTone(run.status), color) + raw.slice(statusWidth) + paint(scrollbar, 'muted', color));
  }
  for (const detail of details) lines.push(cell(detail, width, 'muted', color));
  lines.push(paint('─'.repeat(width), 'muted', color), inspectorFooter(width, color, false), cell(inspectorMessage(state), width, state.stale ? 'yellow' : 'muted', color));
  if (state.viewMenu !== undefined) {
    const start = Math.min(6, Math.max(0, height - 10));
    const menuWidth = Math.min(width, 32);
    INSPECTOR_VIEWS.forEach((view, index) => {
      if (start + index < height - 2) lines[start + index] = ' '.repeat(width - menuWidth) + cell(` ${index + 1} ${title(view)} `, menuWidth, state.viewMenu === index ? 'selected' : 'button', color);
    });
  }
  return frame(lines, height, width);
}

export function analyticsContent(state: InspectorRenderState, width: number, color: boolean): string[] {
  const data = state.analytics;
  if (data === undefined) return [
    cell('WORKFLOW / ANALYTICS', width, 'green', color), '',
    fit(state.analyticsWorkflowId ?? '', width), '',
    fit(state.analyticsError ?? 'Loading workflow analytics…', width),
  ];
  const { workflow, totals: t, coverage } = data;
  const lines: string[] = [''];
  const split = (left: string, right: string, tone: Tone = 'muted'): string => {
    const rightWidth = Math.min(Math.floor(width / 2), Bun.stringWidth(safeText(right)));
    return cell(left, width - rightWidth, tone, color) + cell(right, rightWidth, 'muted', color, true);
  };
  const rule = (): void => { lines.push(paint('─'.repeat(width), 'muted', color)); };
  lines.push(split('WORKFLOW / ANALYTICS', 'Last 24 hours · UTC', 'green'), '',
    cell(workflow.name ?? workflow.workflowId, width, 'white', color),
    cell(workflow.description ?? '', width, 'muted', color), '',
    fit(`File  ${workflow.file ?? '—'}    Trigger  ${workflow.triggerTypes.join(', ') || '—'}`, width), '');
  rule();
  const finished = t.succeeded + t.failed;
  const stats = [
    ['Finished runs', String(finished), `${t.succeeded} succeeded · ${t.failed} failed`, 'white'],
    ['Success rate', finished === 0 ? '—' : `${(t.succeeded / finished * 100).toFixed(1)}%`, 'Of finished runs', 'green'],
    ['Avg. duration', elapsed(t.averageDurationMs), `p95 ${elapsed(t.p95DurationMs)}`, 'white'],
    ['Retried runs', coverage.detailedRuns === 0 ? '—' : String(t.retriedRuns), `${t.retryAttempts} retry attempts`, 'yellow'],
  ] as const;
  if (width >= 90) {
    const span = Math.floor((width - 3) / 4);
    for (let line = 0; line < 3; line++) {
      lines.push(stats.map((stat, index) => cell(stat[line]!, index === 3 ? width - (span + 1) * 3 : span, line === 1 ? stat[3] : 'muted', color) + (index < 3 ? paint('│', 'muted', color) : '')).join(''));
    }
  } else {
    for (const stat of stats) lines.push(cell(`${stat[0]}: ${stat[1]}`, width, stat[3], color), cell(`  ${stat[2]}`, width, 'muted', color));
  }
  rule(); lines.push('', split('RUN ACTIVITY', '4-hour intervals', 'white'));
  const timeWidth = Math.max(1, width - 33);
  lines.push(cell('Time window', timeWidth, 'muted', color) + cell('Succeeded', 11, 'muted', color, true) + cell('Failed', 11, 'muted', color, true) + cell('Total', 11, 'muted', color, true));
  rule();
  for (const bucket of data.activity) {
    lines.push(fit(`${bucket.startAt.slice(11, 16)}–${bucket.endAt.slice(11, 16)}`, timeWidth) + cell(String(bucket.succeeded), 11, bucket.succeeded ? 'green' : 'muted', color, true) + cell(String(bucket.failed), 11, bucket.failed ? 'red' : 'muted', color, true) + cell(String(bucket.total), 11, 'white', color, true));
  }
  lines.push(''); rule(); lines.push('');
  const stepLines = (size: number): string[] => {
    const nameWidth = Math.max(1, size - 24);
    return [
      cell('STEP PERFORMANCE', size, 'white', color),
      cell('Step', nameWidth, 'muted', color) + cell('Avg.', 10, 'muted', color, true) + cell('Fail', 7, 'muted', color, true) + cell('Retry', 7, 'muted', color, true),
      paint('─'.repeat(size), 'muted', color),
      ...(data.steps.length ? data.steps.map(step => fit(step.name ?? step.nodeId, nameWidth) + fit(elapsed(step.averageDurationMs), 10, 'right') + cell(String(step.failures), 7, step.failures ? 'red' : 'muted', color, true) + cell(String(step.retries), 7, step.retries ? 'yellow' : 'muted', color, true)) : [cell('No completed step details available.', size, 'muted', color)]),
      '', cell(`Median ${elapsed(t.medianDurationMs)}  Longest ${elapsed(t.longestDurationMs)}`, size, 'muted', color),
    ];
  };
  const failureLines = (size: number): string[] => [
    cell('RECENT FAILURES', size, 'white', color),
    ...(data.failures.length ? data.failures.flatMap(item => [
      fit(`${item.at.slice(11, 16)}  ${item.nodeId}`, size),
      cell(`       ${item.code}`, size, 'red', color), '',
    ]) : [cell('No failures in detailed runs.', size, 'green', color), '']),
    cell(`NOW ${t.active} active  ${t.queued} queued  ${t.waiting} waiting`, size, 'muted', color),
  ];
  if (width >= 90) {
    const leftWidth = Math.floor(width * 0.56), rightWidth = width - leftWidth - 3;
    const left = stepLines(leftWidth), right = failureLines(rightWidth);
    for (let i = 0; i < Math.max(left.length, right.length); i++) lines.push((left[i] ?? ' '.repeat(leftWidth)) + '   ' + (right[i] ?? ' '.repeat(rightWidth)));
  } else lines.push(...stepLines(width), '', ...failureLines(width));
  lines.push('', cell(`Runs: ${coverage.runs} in window${coverage.runsLimited ? ' · limited to latest 200 updated runs' : ''} · ${t.cancelled} cancelled (excluded from finished)`, width, coverage.runsLimited ? 'yellow' : 'muted', color),
    cell(`Step/retry/failure details: latest ${coverage.detailedRuns} finished runs${coverage.detailsLimited ? ' · partial coverage' : ''}${coverage.unavailableRuns ? ` · ${coverage.unavailableRuns} unavailable` : ''}`, width, coverage.detailsLimited ? 'yellow' : 'muted', color),
    cell(`Captured ${data.capturedAt} · cached up to 15s`, width, 'muted', color));
  return lines;
}

export function renderAnalyticsDashboard(state: InspectorRenderState, columns: number, rows: number, color: boolean): string {
  const width = Math.max(1, Math.floor(columns)), height = Math.max(1, Math.floor(rows));
  // Small terminals use a readable stacked view; scroll never hides the footer.
  const bodyWidth = Math.max(1, width - 1), bodyHeight = Math.max(1, height - 3);
  const content = analyticsContent(state, bodyWidth, color);
  const offset = Math.max(0, Math.min(state.analyticsScroll ?? 0, content.length - bodyHeight));
  const thumb = content.length > bodyHeight ? Math.round(offset / Math.max(1, content.length - bodyHeight) * (bodyHeight - 1)) : -1;
  const lines = Array.from({ length: bodyHeight }, (_, index) => (content[offset + index] ?? ' '.repeat(bodyWidth)) + paint(thumb < 0 ? ' ' : index === thumb ? '┃' : '│', 'muted', color));
  lines.push(paint('─'.repeat(width), 'muted', color), inspectorFooter(width, color, true), cell(state.analyticsError ?? inspectorMessage(state), width, state.analyticsError || state.stale ? 'yellow' : 'muted', color));
  return frame(lines, height, width);
}
