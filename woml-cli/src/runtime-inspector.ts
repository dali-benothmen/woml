import { randomUUID } from 'node:crypto';
import { resolve } from 'node:path';

import {
  readRuntimeDescriptor,
  requestRuntimeOperation,
  runtimeDescriptorPath,
  type RuntimeDescriptorV1,
} from './runtime-control';
import {
  consumeOperationsStream,
  type OperationsStreamEventV1,
} from './operations-stream';

export const inspectUsage =
  'Usage: woml inspect [--state <path>] [--no-color]';

export type InspectorView =
  | 'overview'
  | 'runs'
  | 'triggers'
  | 'approvals'
  | 'queues'
  | 'failures'
  | 'runtime';

export interface InspectorWorkflowV1 {
  readonly workflowId: string;
  readonly definitionHash: string;
  readonly active: number;
  readonly queued: number;
  readonly waiting: number;
  readonly failed: number;
  readonly triggerTypes: readonly string[];
}

export interface InspectorRunV1 {
  readonly runId: string;
  readonly workflowId: string;
  readonly status:
    | 'queued'
    | 'running'
    | 'waiting'
    | 'retrying'
    | 'succeeded'
    | 'failed'
    | 'cancelled';
  readonly durationMs: number;
  readonly currentNodeId?: string;
  readonly parentRunId?: string;
}

export interface InspectorComponentV1 {
  readonly name: string;
  readonly kind: string;
  readonly status: 'ready' | 'degraded' | 'unready' | 'stopped';
  readonly code?: string;
}

export interface InspectorAlertV1 {
  readonly at: string;
  readonly level: 'warn' | 'error';
  readonly code: string;
  readonly message: string;
}

export interface InspectorSnapshotV1 {
  readonly profile: 'woml.runtime-operations-snapshot/v1';
  readonly runtimeInstanceId: string;
  readonly sequence: number;
  readonly capturedAt: string;
  readonly lifecycle: string;
  readonly ready: boolean;
  readonly uptimeMs: number;
  readonly workflows: readonly InspectorWorkflowV1[];
  readonly runs: readonly InspectorRunV1[];
  readonly components: readonly InspectorComponentV1[];
  readonly alerts: readonly InspectorAlertV1[];
}

export type InspectorStreamEventV1 = OperationsStreamEventV1;

export interface InspectorTerminal {
  readonly isTTY: boolean;
  readonly columns: number;
  readonly rows: number;
  write(text: string): void;
  setRawMode(enabled: boolean): void;
  onInput(listener: (text: string) => void): () => void;
  onResize(listener: () => void): () => void;
}

export interface InspectorArguments {
  readonly statePath: string;
  readonly color: boolean;
}

export interface InspectorRenderState {
  readonly view: InspectorView;
  readonly selected: number;
  readonly filter: string;
  readonly searchInput?: string;
  readonly confirmationRunId?: string;
  readonly expanded: boolean;
  readonly showHelp: boolean;
  readonly stale: boolean;
  readonly connectionMessage?: string;
  readonly recentEvents: readonly InspectorStreamEventV1[];
}

const VIEWS: readonly InspectorView[] = [
  'overview',
  'runs',
  'triggers',
  'approvals',
  'queues',
  'failures',
  'runtime',
];
const CSI = '\u001b[';
const RESET = `${CSI}0m`;
const COLORS = {
  cyan: `${CSI}1;36m`,
  green: `${CSI}1;32m`,
  blue: `${CSI}1;34m`,
  yellow: `${CSI}1;33m`,
  magenta: `${CSI}1;35m`,
  red: `${CSI}1;31m`,
  dim: `${CSI}2m`,
  inverse: `${CSI}7m`,
} as const;

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function isString(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0 && value.length <= 2048;
}

function isCount(value: unknown): value is number {
  return Number.isSafeInteger(value) && Number(value) >= 0;
}

function decodeSnapshot(value: unknown): InspectorSnapshotV1 {
  if (
    !isRecord(value) ||
    value.profile !== 'woml.runtime-operations-snapshot/v1' ||
    !isString(value.runtimeInstanceId) ||
    !isCount(value.sequence) ||
    !isString(value.capturedAt) ||
    !isString(value.lifecycle) ||
    typeof value.ready !== 'boolean' ||
    !isCount(value.uptimeMs) ||
    !Array.isArray(value.workflows) ||
    !Array.isArray(value.runs) ||
    !Array.isArray(value.components) ||
    !Array.isArray(value.alerts)
  ) {
    throw new Error('The runtime returned an invalid operations snapshot.');
  }
  return value as unknown as InspectorSnapshotV1;
}

export function parseInspectArguments(args: readonly string[]): InspectorArguments {
  if (args[0] !== 'inspect') throw new Error(inspectUsage);
  let statePath = resolve('.woml/state.sqlite');
  let color = !('NO_COLOR' in process.env);
  const seen = new Set<string>();
  for (let index = 1; index < args.length; index += 1) {
    const option = args[index]!;
    if (seen.has(option)) throw new Error(inspectUsage);
    seen.add(option);
    if (option === '--no-color') {
      color = false;
      continue;
    }
    if (option !== '--state') throw new Error(inspectUsage);
    const value = args[++index];
    if (value === undefined || value.length === 0 || value.startsWith('--')) {
      throw new Error(inspectUsage);
    }
    statePath = resolve(value);
  }
  return { statePath, color };
}

function color(text: string, name: keyof typeof COLORS, enabled: boolean): string {
  return enabled ? `${COLORS[name]}${text}${RESET}` : text;
}

function truncate(value: string, width: number): string {
  if (width <= 0) return '';
  const points = [...value.replace(/[\u0000-\u001f\u007f]/g, ' ')];
  if (points.length <= width) return points.join('');
  if (width === 1) return '…';
  return `${points.slice(0, width - 1).join('')}…`;
}

function pad(value: string, width: number): string {
  const safe = truncate(value, width);
  return safe + ' '.repeat(Math.max(0, width - [...safe].length));
}

function duration(milliseconds: number): string {
  if (milliseconds < 1000) return `${Math.round(milliseconds)}ms`;
  if (milliseconds < 60_000) return `${(milliseconds / 1000).toFixed(1)}s`;
  if (milliseconds < 3_600_000) return `${Math.floor(milliseconds / 60_000)}m`;
  if (milliseconds < 86_400_000) return `${Math.floor(milliseconds / 3_600_000)}h`;
  return `${Math.floor(milliseconds / 86_400_000)}d`;
}

function statusColor(status: string): keyof typeof COLORS {
  if (status === 'ready' || status === 'succeeded' || status === 'healthy') return 'green';
  if (status === 'running') return 'blue';
  if (status === 'queued') return 'magenta';
  if (status === 'waiting' || status === 'retrying' || status === 'degraded') return 'yellow';
  if (status === 'failed' || status === 'error' || status === 'unready') return 'red';
  return 'dim';
}

function selectedRuns(snapshot: InspectorSnapshotV1, state: InspectorRenderState): readonly InspectorRunV1[] {
  let runs = snapshot.runs;
  if (state.view === 'approvals') runs = runs.filter(run => run.status === 'waiting');
  if (state.view === 'queues') runs = runs.filter(run => ['queued', 'retrying'].includes(run.status));
  if (state.view === 'failures') runs = runs.filter(run => run.status === 'failed');
  const query = state.filter.toLocaleLowerCase();
  return query.length === 0
    ? runs
    : runs.filter(run => `${run.workflowId} ${run.runId} ${run.status}`.toLocaleLowerCase().includes(query));
}

function runLines(
  snapshot: InspectorSnapshotV1,
  state: InspectorRenderState,
  width: number
): { lines: string[]; colors: (keyof typeof COLORS | undefined)[] } {
  const lines: string[] = [];
  const colors: (keyof typeof COLORS | undefined)[] = [];
  const runs = selectedRuns(snapshot, state);
  const workflowWidth = Math.max(12, Math.min(28, Math.floor(width * 0.28)));
  const idWidth = Math.max(14, Math.min(32, width - workflowWidth - 28));
  lines.push(`${pad('STATUS', 11)} ${pad('WORKFLOW', workflowWidth)} ${pad('RUN ID', idWidth)} ${pad('WORK', 16)} DURATION`);
  colors.push('cyan');
  if (runs.length === 0) {
    lines.push(state.filter.length > 0 ? 'No runs match the current filter.' : 'No runs in this view.');
    colors.push('dim');
    return { lines, colors };
  }
  for (const [index, run] of runs.entries()) {
    const marker = index === Math.min(state.selected, runs.length - 1) ? '›' : ' ';
    lines.push(
      `${marker}${pad(run.status, 10)} ${pad(run.workflowId, workflowWidth)} ${pad(run.runId, idWidth)} ${pad(run.currentNodeId ?? '—', 16)} ${duration(run.durationMs)}`
    );
    colors.push(index === Math.min(state.selected, runs.length - 1) ? 'inverse' : statusColor(run.status));
  }
  if (state.expanded) {
    const run = runs[Math.min(state.selected, runs.length - 1)];
    if (run !== undefined) {
      lines.push('');
      colors.push(undefined);
      lines.push(`Selected: ${run.runId}`);
      colors.push('cyan');
      lines.push(`Execution: ${run.workflowId} → ${run.currentNodeId ?? run.status} [${run.status}]`);
      colors.push(statusColor(run.status));
      if (run.parentRunId !== undefined) {
        lines.push(`Called by: ${run.parentRunId}`);
        colors.push('dim');
      }
    }
  }
  return { lines, colors };
}

function workflowLines(snapshot: InspectorSnapshotV1, state: InspectorRenderState, width: number) {
  const lines: string[] = [];
  const colors: (keyof typeof COLORS | undefined)[] = [];
  const idWidth = Math.max(16, Math.min(36, width - 42));
  lines.push(`${pad('WORKFLOW', idWidth)} ACTIVE  QUEUED  WAITING  FAILED  TRIGGERS`);
  colors.push('cyan');
  const query = state.filter.toLocaleLowerCase();
  const workflows = snapshot.workflows.filter(item =>
    query.length === 0 || `${item.workflowId} ${item.triggerTypes.join(' ')}`.toLocaleLowerCase().includes(query)
  );
  for (const workflow of workflows) {
    lines.push(
      `${pad(workflow.workflowId, idWidth)} ${pad(String(workflow.active), 7)} ${pad(String(workflow.queued), 7)} ${pad(String(workflow.waiting), 8)} ${pad(String(workflow.failed), 7)} ${workflow.triggerTypes.join(', ') || '—'}`
    );
    colors.push(workflow.failed > 0 ? 'red' : workflow.active > 0 ? 'blue' : undefined);
  }
  if (workflows.length === 0) {
    lines.push(state.filter.length > 0 ? 'No workflows match the current filter.' : 'No workflows loaded.');
    colors.push('dim');
  }
  return { lines, colors };
}

function triggerLines(snapshot: InspectorSnapshotV1, state: InspectorRenderState) {
  const lines = ['WORKFLOW                       TRIGGERS'];
  const colors: (keyof typeof COLORS | undefined)[] = ['cyan'];
  const query = state.filter.toLocaleLowerCase();
  for (const workflow of snapshot.workflows) {
    const value = `${workflow.workflowId} ${workflow.triggerTypes.join(' ')}`;
    if (query.length > 0 && !value.toLocaleLowerCase().includes(query)) continue;
    lines.push(`${pad(workflow.workflowId, 30)} ${workflow.triggerTypes.join(', ') || 'manual only'}`);
    colors.push('green');
  }
  const triggerComponents = snapshot.components.filter(component => component.kind === 'trigger' || component.kind === 'provider');
  if (triggerComponents.length > 0) {
    lines.push('', 'PROVIDER / TRIGGER HOST         STATUS');
    colors.push(undefined, 'cyan');
    for (const component of triggerComponents) {
      lines.push(`${pad(component.name, 31)} ${component.status}${component.code === undefined ? '' : ` (${component.code})`}`);
      colors.push(statusColor(component.status));
    }
  }
  return { lines, colors };
}

function runtimeLines(snapshot: InspectorSnapshotV1) {
  const lines = ['COMPONENT                      KIND          STATUS'];
  const colors: (keyof typeof COLORS | undefined)[] = ['cyan'];
  for (const component of snapshot.components) {
    lines.push(`${pad(component.name, 30)} ${pad(component.kind, 13)} ${component.status}${component.code === undefined ? '' : ` (${component.code})`}`);
    colors.push(statusColor(component.status));
  }
  if (snapshot.components.length === 0) {
    lines.push('No runtime components reported.');
    colors.push('dim');
  }
  lines.push('', `Runtime ID: ${snapshot.runtimeInstanceId}`, `Captured: ${snapshot.capturedAt}`);
  colors.push(undefined, 'dim', 'dim');
  return { lines, colors };
}

function failureLines(snapshot: InspectorSnapshotV1, state: InspectorRenderState, width: number) {
  const runs = runLines(snapshot, state, width);
  const lines = [...runs.lines, '', 'RECENT ALERTS'];
  const colors: (keyof typeof COLORS | undefined)[] = [...runs.colors, undefined, 'cyan'];
  for (const alert of snapshot.alerts.slice(-20).reverse()) {
    lines.push(`${alert.at.slice(11, 19)} ${pad(alert.code, 34)} ${alert.message}`);
    colors.push(alert.level === 'error' ? 'red' : 'yellow');
  }
  if (snapshot.alerts.length === 0) {
    lines.push('No runtime alerts.');
    colors.push('green');
  }
  return { lines, colors };
}

function helpLines(): { lines: string[]; colors: (keyof typeof COLORS | undefined)[] } {
  return {
    lines: [
      'KEYBOARD HELP',
      '',
      'Tab / Shift+Tab   change view',
      '1–7               open a view directly',
      '↑ / ↓             select a run',
      'Enter             expand selected run',
      '/                 search workflows and runs',
      'l                 show recent live events',
      'c                 cancel selected active run',
      'r                 refresh now',
      '?                 close help',
      'q                 quit inspector (workflows keep running)',
    ],
    colors: ['cyan', undefined, ...Array(10).fill(undefined)],
  };
}

function eventLines(events: readonly InspectorStreamEventV1[]) {
  const lines = ['RECENT LIVE EVENTS'];
  const colors: (keyof typeof COLORS | undefined)[] = ['cyan'];
  for (const event of events.slice(-30).reverse()) {
    lines.push(`${event.occurredAt.slice(11, 19)} ${pad(event.kind, 14)} ${pad(event.subject.id, 28)} ${event.subject.status}${event.subject.code === undefined ? '' : ` (${event.subject.code})`}`);
    colors.push(event.subject.code === undefined ? statusColor(event.subject.status) : 'red');
  }
  if (events.length === 0) {
    lines.push('Waiting for live runtime activity…');
    colors.push('dim');
  }
  return { lines, colors };
}

export function renderInspectorFrame(
  snapshot: InspectorSnapshotV1,
  state: InspectorRenderState,
  columns: number,
  rows: number,
  colorsEnabled: boolean
): string {
  const width = Math.max(40, columns);
  const height = Math.max(12, rows);
  const active = snapshot.runs.filter(run => ['running', 'retrying'].includes(run.status)).length;
  const queued = snapshot.runs.filter(run => run.status === 'queued').length;
  const waiting = snapshot.runs.filter(run => run.status === 'waiting').length;
  const health = snapshot.ready ? 'healthy' : snapshot.lifecycle;
  const header = ` WOML INSPECT  ${health}  uptime ${duration(snapshot.uptimeMs)}  workflows ${snapshot.workflows.length}  active ${active}  queued ${queued}  waiting ${waiting}${state.stale ? '  STALE' : ''}`;
  const tabs = VIEWS.map(view => view === state.view ? `[${view.toUpperCase()}]` : view).join('  ');
  let content:
    | { lines: string[]; colors: (keyof typeof COLORS | undefined)[] };
  if (state.showHelp) content = helpLines();
  else if (state.view === 'overview') content = workflowLines(snapshot, state, width);
  else if (state.view === 'triggers') content = triggerLines(snapshot, state);
  else if (state.view === 'runtime') content = runtimeLines(snapshot);
  else if (state.view === 'failures') content = failureLines(snapshot, state, width);
  else content = runLines(snapshot, state, width);
  if (state.expanded && state.view === 'overview') content = eventLines(state.recentEvents);

  const lines: string[] = [
    color(truncate(header, width), snapshot.ready ? 'green' : 'red', colorsEnabled),
    color('─'.repeat(width), 'cyan', colorsEnabled),
    color(truncate(tabs, width), 'cyan', colorsEnabled),
    color('─'.repeat(width), 'cyan', colorsEnabled),
  ];
  const bodyHeight = Math.max(1, height - 7);
  for (let index = 0; index < Math.min(bodyHeight, content.lines.length); index += 1) {
    const line = truncate(content.lines[index] ?? '', width);
    const tone = content.colors[index];
    lines.push(tone === undefined ? line : color(line, tone, colorsEnabled));
  }
  while (lines.length < height - 3) lines.push('');
  lines.push(color('─'.repeat(width), 'cyan', colorsEnabled));
  const message = state.confirmationRunId === undefined
    ? state.searchInput === undefined
      ? state.connectionMessage ?? `↑↓ select  Tab views  Enter details  / search  l events  c cancel  ? help  q quit`
      : `Search: ${state.searchInput}█  (Enter apply, Esc cancel)`
    : `Cancel ${state.confirmationRunId}? Press y to confirm or n to keep it running.`;
  lines.push(color(truncate(message, width), state.confirmationRunId === undefined ? 'dim' : 'yellow', colorsEnabled));
  if (state.filter.length > 0) lines.push(color(truncate(`Filter: ${state.filter}`, width), 'yellow', colorsEnabled));
  else lines.push('');
  return `${CSI}H${lines.slice(0, height).join('\n')}`;
}

export function createProcessInspectorTerminal(): InspectorTerminal {
  return {
    isTTY: process.stdin.isTTY === true && process.stdout.isTTY === true,
    get columns() { return process.stdout.columns ?? 80; },
    get rows() { return process.stdout.rows ?? 24; },
    write: text => process.stdout.write(text),
    setRawMode: enabled => process.stdin.setRawMode?.(enabled),
    onInput: listener => {
      const receive = (data: Buffer | string): void => listener(data.toString());
      process.stdin.on('data', receive);
      process.stdin.resume();
      return () => process.stdin.off('data', receive);
    },
    onResize: listener => {
      process.stdout.on('resize', listener);
      return () => process.stdout.off('resize', listener);
    },
  };
}

async function fetchSnapshot(descriptor: RuntimeDescriptorV1, fetcher: typeof fetch): Promise<InspectorSnapshotV1> {
  const response = await fetcher(`${descriptor.adminUrl}/v1/snapshot`, {
    headers: { authorization: `Bearer ${descriptor.capability}` },
  });
  if (!response.ok) throw new Error(`Runtime snapshot request failed with HTTP ${response.status}.`);
  return decodeSnapshot(await response.json());
}

export async function runRuntimeInspector(options: {
  readonly args: InspectorArguments;
  readonly terminal?: InspectorTerminal;
  readonly fetcher?: typeof fetch;
  readonly readDescriptor?: (path: string) => Promise<RuntimeDescriptorV1>;
}): Promise<number> {
  const terminal = options.terminal ?? createProcessInspectorTerminal();
  if (!terminal.isTTY) {
    terminal.write('WOML inspect requires an interactive terminal. For scriptable output, use: woml list --json\n');
    return 2;
  }
  const fetcher = options.fetcher ?? globalThis.fetch;
  const readDescriptor = options.readDescriptor ?? readRuntimeDescriptor;
  const descriptorFile = runtimeDescriptorPath(options.args.statePath);
  let descriptor: RuntimeDescriptorV1;
  let snapshot: InspectorSnapshotV1;
  try {
    descriptor = await readDescriptor(descriptorFile);
    snapshot = await fetchSnapshot(descriptor, fetcher);
  } catch (error) {
    terminal.write(`WOML inspect could not connect to an active runtime. Start it with "woml run <workflow.woml>" or pass its state file with --state.\n${error instanceof Error ? error.message : String(error)}\n`);
    return 1;
  }

  let state: InspectorRenderState = {
    view: 'overview', selected: 0, filter: '', expanded: false,
    showHelp: false, stale: false, recentEvents: [],
  };
  let closed = false;
  let refreshPending = false;
  let eventRefreshTimer: ReturnType<typeof setTimeout> | undefined;
  let renderTimer: ReturnType<typeof setTimeout> | undefined;
  let lastRenderedAt = 0;
  let renderFailed = false;
  let resolveDone!: () => void;
  const done = new Promise<void>(resolveDoneValue => { resolveDone = resolveDoneValue; });
  const streamAbort = new AbortController();
  const renderNow = (): void => {
    try {
      terminal.write(renderInspectorFrame(snapshot, state, terminal.columns, terminal.rows, options.args.color));
      lastRenderedAt = Date.now();
    } catch {
      renderFailed = true;
      closed = true;
      streamAbort.abort();
      resolveDone();
    }
  };
  const render = (): void => {
    if (closed) return;
    const remaining = 100 - (Date.now() - lastRenderedAt);
    if (remaining <= 0) {
      if (renderTimer !== undefined) clearTimeout(renderTimer);
      renderTimer = undefined;
      renderNow();
      return;
    }
    if (renderTimer !== undefined) return;
    renderTimer = setTimeout(() => {
      renderTimer = undefined;
      if (!closed) renderNow();
    }, remaining);
  };
  const refresh = async (): Promise<void> => {
    if (closed || refreshPending) return;
    refreshPending = true;
    try {
      descriptor = await readDescriptor(descriptorFile);
      snapshot = await fetchSnapshot(descriptor, fetcher);
      state = { ...state, stale: false, connectionMessage: undefined };
    } catch {
      state = { ...state, stale: true, connectionMessage: 'Disconnected; reconnecting without affecting workflows…' };
    } finally {
      refreshPending = false;
      if (!closed) render();
    }
  };
  const quit = (): void => {
    if (closed) return;
    closed = true;
    streamAbort.abort();
    resolveDone();
  };
  const moveSelection = (delta: number): void => {
    const count = selectedRuns(snapshot, state).length;
    state = { ...state, selected: Math.max(0, Math.min(Math.max(0, count - 1), state.selected + delta)) };
  };
  const input = async (text: string): Promise<void> => {
    if (state.searchInput !== undefined) {
      if (text === '\r' || text === '\n') state = { ...state, filter: state.searchInput, searchInput: undefined, selected: 0 };
      else if (text === '\u001b') state = { ...state, searchInput: undefined };
      else if (text === '\u007f') state = { ...state, searchInput: state.searchInput.slice(0, -1) };
      else if (!text.startsWith('\u001b') && /^[\p{L}\p{N} _.:/-]+$/u.test(text)) state = { ...state, searchInput: `${state.searchInput}${text}`.slice(0, 120) };
      render();
      return;
    }
    if (state.confirmationRunId !== undefined) {
      if (text.toLocaleLowerCase() === 'y') {
        const runId = state.confirmationRunId;
        state = { ...state, confirmationRunId: undefined, connectionMessage: `Requesting cancellation for ${runId}…` };
        render();
        try {
          descriptor = await readDescriptor(descriptorFile);
          const result = await requestRuntimeOperation(descriptor, 'cancel_run', runId, fetcher, `cancel_${randomUUID().replaceAll('-', '')}`);
          state = { ...state, connectionMessage: result.code ?? `Cancellation requested for ${runId}.` };
          await refresh();
        } catch (error) {
          state = { ...state, connectionMessage: `Cancellation failed: ${error instanceof Error ? error.message : String(error)}` };
          render();
        }
      } else if (text.toLocaleLowerCase() === 'n' || text === '\u001b') {
        state = { ...state, confirmationRunId: undefined, connectionMessage: 'Cancellation dismissed.' };
        render();
      }
      return;
    }
    if (text === 'q' || text === '\u0003') quit();
    else if (text === '\t') state = { ...state, view: VIEWS[(VIEWS.indexOf(state.view) + 1) % VIEWS.length]!, selected: 0, expanded: false };
    else if (text === '\u001b[Z') state = { ...state, view: VIEWS[(VIEWS.indexOf(state.view) + VIEWS.length - 1) % VIEWS.length]!, selected: 0, expanded: false };
    else if (text === '\u001b[A') moveSelection(-1);
    else if (text === '\u001b[B') moveSelection(1);
    else if (/^[1-7]$/.test(text)) state = { ...state, view: VIEWS[Number(text) - 1]!, selected: 0, expanded: false };
    else if (text === '\r' || text === '\n') state = { ...state, expanded: !state.expanded };
    else if (text === '/') state = { ...state, searchInput: state.filter };
    else if (text === 'l') state = { ...state, view: 'overview', expanded: true, showHelp: false };
    else if (text === '?') state = { ...state, showHelp: !state.showHelp };
    else if (text === 'r') await refresh();
    else if (text === 'c') {
      const run = selectedRuns(snapshot, state)[state.selected];
      if (run !== undefined && ['queued', 'running', 'waiting', 'retrying'].includes(run.status)) state = { ...state, confirmationRunId: run.runId };
      else state = { ...state, connectionMessage: 'Select an active run before requesting cancellation.' };
    }
    if (!closed) render();
  };

  terminal.write(`${CSI}?1049h${CSI}?25l${CSI}2J`);
  terminal.setRawMode(true);
  const removeInput = terminal.onInput(value => { void input(value); });
  const removeResize = terminal.onResize(render);
  const stopOnSignal = (): void => quit();
  process.once('SIGINT', stopOnSignal);
  process.once('SIGTERM', stopOnSignal);
  render();

  const streamLoop = (async () => {
    let delay = 250;
    while (!closed) {
      try {
        descriptor = await readDescriptor(descriptorFile);
        const response = await fetcher(`${descriptor.adminUrl}/v1/stream?after=${snapshot.sequence}`, {
          headers: { authorization: `Bearer ${descriptor.capability}` },
          signal: streamAbort.signal,
        });
        delay = 250;
        await consumeOperationsStream(response, event => {
          if (event.subject.code === 'WOML_OBSERVABILITY_STREAM_GAP') {
            state = { ...state, stale: true, connectionMessage: 'Live updates were missed; resynchronizing…' };
            void refresh();
            return;
          }
          state = { ...state, recentEvents: [...state.recentEvents, event].slice(-200), connectionMessage: undefined };
          render();
          if (eventRefreshTimer === undefined) {
            eventRefreshTimer = setTimeout(() => {
              eventRefreshTimer = undefined;
              void refresh();
            }, 500);
          }
        }, streamAbort.signal);
        if (!closed) throw new Error('Runtime event stream closed.');
      } catch {
        if (closed) break;
        state = { ...state, stale: true, connectionMessage: 'Live connection lost; reconnecting…' };
        render();
        await Bun.sleep(delay);
        delay = Math.min(5000, delay * 2);
        await refresh();
      }
    }
  })();
  const periodic = setInterval(() => { void refresh(); }, 5000);
  try {
    await done;
  } finally {
    closed = true;
    clearInterval(periodic);
    if (eventRefreshTimer !== undefined) clearTimeout(eventRefreshTimer);
    if (renderTimer !== undefined) clearTimeout(renderTimer);
    streamAbort.abort();
    await streamLoop.catch(() => {});
    removeInput();
    removeResize();
    process.off('SIGINT', stopOnSignal);
    process.off('SIGTERM', stopOnSignal);
    try { terminal.setRawMode(false); } catch { /* best-effort restoration */ }
    try { terminal.write(`${CSI}0m${CSI}?25h${CSI}?1049l`); } catch { /* output is already unavailable */ }
  }
  return renderFailed ? 1 : 0;
}
