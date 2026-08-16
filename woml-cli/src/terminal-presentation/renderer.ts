import type {
  ColorMode,
  HumanPresentationFormat,
  JsonValue,
  LifecyclePresentationV1,
  PresentationRenderOptions,
  RunPresentationStatus,
  RunPresentationV1,
  StepPresentationKind,
  StepPresentationStatus,
  StepPresentationV1,
  TriggerPresentationV1,
  WorkflowPresentationV1,
} from './types';

const CSI = '\u001b[';
const RESET = `${CSI}0m`;
const ANSI = {
  bold: `${CSI}1m`,
  dim: `${CSI}2m`,
  cyan: `${CSI}36m`,
  green: `${CSI}32m`,
  blue: `${CSI}34m`,
  yellow: `${CSI}33m`,
  magenta: `${CSI}35m`,
  red: `${CSI}31m`,
} as const;

const SENSITIVE_KEYS = [
  'authorization', 'cookie', 'setcookie', 'password', 'passwd', 'secret',
  'token', 'apikey', 'accesskey', 'privatekey', 'credential',
  'idempotencykey', 'capability', 'approvalurl', 'resumeurl',
] as const;
const MAX_PREVIEW_DEPTH = 5;
const MAX_PREVIEW_PROPERTIES = 20;
const MAX_PREVIEW_ARRAY_ITEMS = 20;
const MAX_PREVIEW_STRING = 500;
const MAX_PREVIEW_NODES = 2000;

type ColorName = keyof typeof ANSI;

interface ResolvedRenderOptions {
  readonly format: HumanPresentationFormat | 'json';
  readonly color: boolean;
  readonly width: number;
  readonly unicode: boolean;
  readonly locale: string;
  readonly timeZone?: string;
  readonly fullResultCommand: (runId: string) => string;
  readonly manualInstruction: string;
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.max(minimum, Math.min(maximum, Math.floor(value)));
}

function resolveOptions(options: PresentationRenderOptions = {}): ResolvedRenderOptions {
  const environment = options.environment ?? process.env;
  const format = options.format ?? 'tty';
  const colorMode: ColorMode = options.color ?? 'auto';
  const terminalSupportsColor =
    format === 'tty' &&
    (options.isTTY ?? true) &&
    !Object.prototype.hasOwnProperty.call(environment, 'NO_COLOR') &&
    environment.TERM !== 'dumb';
  return {
    format,
    color: format === 'tty' && (colorMode === 'always' || (colorMode === 'auto' && terminalSupportsColor)),
    width: clamp(options.width ?? 72, 32, 160),
    unicode: options.unicode ?? environment.TERM !== 'dumb',
    locale: options.locale ?? 'en-GB',
    timeZone: options.timeZone,
    fullResultCommand: options.fullResultCommand ?? (runId => `woml get ${runId} --json`),
    manualInstruction: options.manualInstruction ?? 'Press Enter to start a run',
  };
}

function paint(value: string, color: ColorName, enabled: boolean): string {
  return enabled ? `${ANSI[color]}${value}${RESET}` : value;
}

export function stripAnsi(value: string): string {
  return value
    .replace(/\u001b\][^\u0007]*(?:\u0007|\u001b\\)/gu, '')
    .replace(/\u001b\[[0-?]*[ -/]*[@-~]/gu, '')
    .replace(/\u001b./gu, '');
}

export function sanitizeTerminalText(value: string): string {
  const withoutAnsi = stripAnsi(value).replace(/\r\n?/gu, '\n');
  let safe = '';
  for (const character of withoutAnsi) {
    const code = character.codePointAt(0)!;
    if (code === 9 || code === 10) {
      safe += character;
    } else if (
      code < 32 || code === 127 || (code >= 0x80 && code <= 0x9f) ||
      (code >= 0x202a && code <= 0x202e) ||
      (code >= 0x2066 && code <= 0x2069)
    ) {
      safe += ' ';
    } else if (code === 0x2028 || code === 0x2029) {
      safe += '\n';
    } else {
      safe += character;
    }
  }
  return safe;
}

function sensitiveKey(key: string): boolean {
  const normalized = key.replace(/[-_\s]/gu, '').toLowerCase();
  return SENSITIVE_KEYS.some(candidate =>
    normalized === candidate || normalized.endsWith(candidate)
  );
}

function redactSecretFragments(value: string): string {
  return value
    .replace(/\b(?:Bearer|Basic)\s+[^\s"'\]}>,;]+/giu, match =>
      `${match.slice(0, match.indexOf(' ') + 1)}[redacted]`
    )
    .replace(/\b(?:xox[baprs]|xapp)-[^\s"'\]}>,;]+/giu, '[redacted]')
    .replace(
      /((?:^|[?&\s])(?:token|secret|password|passwd|api[_-]?key|access[_-]?key|private[_-]?key|authorization|idempotency[_-]?key)=)[^&\s"'\]}>,;]+/giu,
      '$1[redacted]'
    );
}

/** Sanitize an untrusted diagnostic before it crosses a terminal/log boundary. */
export function sanitizePresentationDiagnostic(value: string): string {
  return redactSecretFragments(oneLine(value));
}

function oneLine(value: string): string {
  return sanitizeTerminalText(value).replace(/\s+/gu, ' ').trim();
}

function safeJson(
  value: JsonValue,
  key = '',
  depth = 0,
  budget: { nodes: number } = { nodes: 0 }
): JsonValue {
  budget.nodes += 1;
  if (budget.nodes > MAX_PREVIEW_NODES) return '[preview limit reached]';
  if (sensitiveKey(key)) return '[redacted]';
  if (typeof value === 'string') {
    const safe = redactSecretFragments(sanitizeTerminalText(value));
    return safe.length <= MAX_PREVIEW_STRING
      ? safe
      : `${[...safe].slice(0, MAX_PREVIEW_STRING).join('')}…`;
  }
  if (value === null || typeof value === 'boolean' || typeof value === 'number') return value;
  if (depth >= MAX_PREVIEW_DEPTH) return '[maximum depth reached]';
  if (Array.isArray(value)) {
    const items = value.slice(0, MAX_PREVIEW_ARRAY_ITEMS).map(item => safeJson(item, '', depth + 1, budget));
    if (value.length > MAX_PREVIEW_ARRAY_ITEMS) {
      return [...items, `[${value.length - MAX_PREVIEW_ARRAY_ITEMS} more items]`];
    }
    return items;
  }
  const entries = Object.entries(value).slice(0, MAX_PREVIEW_PROPERTIES);
  const safe: Record<string, JsonValue> = {};
  for (const [property, item] of entries) {
    safe[oneLine(property)] = safeJson(item, property, depth + 1, budget);
  }
  const omitted = Object.keys(value).length - entries.length;
  if (omitted > 0) safe['…'] = `[${omitted} more properties]`;
  return safe;
}

export function sanitizePresentation(presentation: RunPresentationV1): RunPresentationV1 {
  const copy = structuredClone(presentation) as RunPresentationV1;
  const cleanString = (value: string | undefined): string | undefined =>
    value === undefined ? undefined : sanitizePresentationDiagnostic(value);
  const cleanFailure = <T extends { readonly code: string; readonly message: string; readonly kind?: string }>(failure: T): T => ({
    ...failure,
    code: sanitizePresentationDiagnostic(failure.code),
    message: sanitizePresentationDiagnostic(failure.message),
    ...(failure.kind === undefined ? {} : { kind: oneLine(failure.kind) }),
  });
  return {
    ...copy,
    workflow: {
      ...copy.workflow,
      id: oneLine(copy.workflow.id),
      name: cleanString(copy.workflow.name),
      description: cleanString(copy.workflow.description),
      version: cleanString(copy.workflow.version),
      definitionHash: oneLine(copy.workflow.definitionHash),
      triggers: copy.workflow.triggers.map(trigger => ({
        ...trigger,
        id: oneLine(trigger.id),
        label: cleanString(trigger.label),
        method: cleanString(trigger.method),
        url: cleanString(trigger.url),
        example: trigger.example === undefined ? undefined : sanitizeTerminalText(trigger.example),
        schedule: cleanString(trigger.schedule),
        timezone: cleanString(trigger.timezone),
        interval: cleanString(trigger.interval),
        event: cleanString(trigger.event),
        workspace: cleanString(trigger.workspace),
        scope: cleanString(trigger.scope),
        warning: cleanString(trigger.warning),
      })),
    },
    runId: oneLine(copy.runId),
    trigger: { ...copy.trigger, id: oneLine(copy.trigger.id) },
    steps: copy.steps.map(step => ({
      ...step,
      id: oneLine(step.id),
      name: cleanString(step.name),
      description: cleanString(step.description),
      detail: cleanString(step.detail),
      ...(step.result === undefined ? {} : { result: safeJson(step.result) }),
      ...(step.failure === undefined ? {} : { failure: cleanFailure(step.failure) }),
    })),
    lifecycle: copy.lifecycle.map(item => ({
      ...item,
      provider: cleanString(item.provider),
      detail: cleanString(item.detail),
      ...(item.failure === undefined ? {} : { failure: cleanFailure(item.failure) }),
    })),
    ...(copy.result === undefined ? {} : { result: safeJson(copy.result) }),
    ...(copy.failure === undefined ? {} : { failure: cleanFailure(copy.failure) }),
    warnings: copy.warnings.map(cleanFailure),
  };
}

function terminalCellWidth(character: string): number {
  const code = character.codePointAt(0)!;
  if (/\p{Mark}/u.test(character) || code === 0x200d || code === 0xfe0f) return 0;
  if (
    code >= 0x1100 && (
      code <= 0x115f || code === 0x2329 || code === 0x232a ||
      (code >= 0x2e80 && code <= 0xa4cf && code !== 0x303f) ||
      (code >= 0xac00 && code <= 0xd7a3) ||
      (code >= 0xf900 && code <= 0xfaff) ||
      (code >= 0xfe10 && code <= 0xfe19) ||
      (code >= 0xfe30 && code <= 0xfe6f) ||
      (code >= 0xff00 && code <= 0xff60) ||
      (code >= 0xffe0 && code <= 0xffe6) ||
      (code >= 0x1f300 && code <= 0x1faff) ||
      (code >= 0x20000 && code <= 0x3fffd)
    )
  ) return 2;
  return 1;
}

function visibleLength(value: string): number {
  return [...stripAnsi(value)].reduce(
    (width, character) => width + terminalCellWidth(character),
    0
  );
}

function sliceToWidth(value: string, width: number): string {
  let result = '';
  let used = 0;
  for (const character of value) {
    const characterWidth = terminalCellWidth(character);
    if (used + characterWidth > width) break;
    result += character;
    used += characterWidth;
  }
  return result;
}

function truncate(value: string, width: number): string {
  const clean = oneLine(value);
  if (width <= 0) return '';
  if (visibleLength(clean) <= width) return clean;
  if (width === 1) return '…';
  return `${sliceToWidth(clean, width - 1)}…`;
}

function padRight(value: string, width: number): string {
  const clipped = truncate(value, width);
  return `${clipped}${' '.repeat(Math.max(0, width - visibleLength(clipped)))}`;
}

function alignEnds(left: string, right: string, width: number): string {
  const rightWidth = Math.min(visibleLength(right), Math.max(0, Math.floor(width / 2)));
  const safeRight = visibleLength(right) <= rightWidth ? right : truncate(right, rightWidth);
  const leftWidth = Math.max(1, width - visibleLength(safeRight) - 1);
  const safeLeft = visibleLength(left) <= leftWidth ? left : truncate(left, leftWidth);
  return `${safeLeft}${' '.repeat(Math.max(1, width - visibleLength(safeLeft) - visibleLength(safeRight)))}${safeRight}`;
}

function wrap(value: string, width: number): string[] {
  const words = oneLine(value).split(' ').filter(Boolean);
  if (words.length === 0) return [];
  const lines: string[] = [];
  let line = '';
  for (const word of words) {
    const chunks: string[] = [];
    let remaining = word;
    while (visibleLength(remaining) > width) {
      const chunk = sliceToWidth(remaining, Math.max(1, width - 1));
      chunks.push(`${chunk}…`);
      remaining = remaining.slice(chunk.length);
    }
    chunks.push(remaining);
    for (const chunk of chunks) {
      if (line.length === 0) line = chunk;
      else if (visibleLength(line) + 1 + visibleLength(chunk) <= width) line += ` ${chunk}`;
      else {
        lines.push(line);
        line = chunk;
      }
    }
  }
  if (line.length > 0) lines.push(line);
  return lines;
}

function duration(milliseconds: number | undefined): string {
  if (milliseconds === undefined) return '—';
  if (milliseconds < 1000) return `${Math.round(milliseconds)} ms`;
  if (milliseconds < 60_000) return `${(milliseconds / 1000).toFixed(milliseconds < 10_000 ? 1 : 0)} s`;
  if (milliseconds < 3_600_000) return `${Math.floor(milliseconds / 60_000)}m ${Math.floor((milliseconds % 60_000) / 1000)}s`;
  return `${Math.floor(milliseconds / 3_600_000)}h ${Math.floor((milliseconds % 3_600_000) / 60_000)}m`;
}

function statusGlyph(status: StepPresentationStatus | RunPresentationStatus, unicode: boolean): string {
  if (status === 'succeeded') return unicode ? '✓' : 'OK';
  if (status === 'failed' || status === 'timed_out') return unicode ? '✗' : 'X';
  if (status === 'cancelled' || status === 'skipped') return unicode ? '○' : '-';
  if (status === 'waiting' || status === 'cancelling') return unicode ? '◐' : 'WAIT';
  if (status === 'retrying') return unicode ? '↻' : 'RETRY';
  if (status === 'queued') return unicode ? '◇' : 'QUEUE';
  if (status === 'finalizing') return unicode ? '◌' : 'FINAL';
  return unicode ? '●' : 'RUN';
}

function statusColor(status: StepPresentationStatus | RunPresentationStatus): ColorName {
  if (status === 'succeeded') return 'green';
  if (status === 'failed' || status === 'timed_out') return 'red';
  if (status === 'waiting' || status === 'retrying' || status === 'cancelling') return 'yellow';
  if (status === 'running' || status === 'finalizing') return 'blue';
  if (status === 'queued') return 'magenta';
  return 'dim';
}

function statusLabel(status: StepPresentationStatus | RunPresentationStatus): string {
  const labels: Record<StepPresentationStatus | RunPresentationStatus, string> = {
    queued: 'Queued',
    running: 'Running',
    waiting: 'Waiting',
    retrying: 'Retrying',
    cancelling: 'Cancelling',
    finalizing: 'Finalizing',
    succeeded: 'Succeeded',
    failed: 'Failed',
    cancelled: 'Cancelled',
    timed_out: 'Timed out',
    skipped: 'Skipped',
  };
  return labels[status];
}

function stepKind(kind: StepPresentationKind): string {
  const labels: Record<StepPresentationKind, string> = {
    step: 'Step',
    script: 'Script',
    custom_step: 'Reusable step',
    switch: 'Switch',
    choose: 'Choose',
    parallel: 'Parallel',
    fork: 'Fork',
    branch: 'Branch',
    approval: 'Approval',
    workflow_call: 'Workflow call',
    workflow_start: 'Workflow start',
  };
  return labels[kind];
}

function formatDate(value: string, options: ResolvedRenderOptions): string {
  const parsed = new Date(value);
  if (!Number.isFinite(parsed.valueOf())) return oneLine(value);
  return new Intl.DateTimeFormat(options.locale, {
    day: '2-digit',
    month: 'short',
    year: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
    ...(options.timeZone === undefined ? {} : { timeZone: options.timeZone }),
  }).format(parsed).replace(',', ' ·');
}

function workflowBox(workflow: WorkflowPresentationV1, options: ResolvedRenderOptions): string[] {
  const width = options.width;
  const inner = width - 2;
  const horizontal = options.unicode ? '─' : '-';
  const corners = options.unicode
    ? { topLeft: '╭', topRight: '╮', bottomLeft: '╰', bottomRight: '╯', vertical: '│' }
    : { topLeft: '+', topRight: '+', bottomLeft: '+', bottomRight: '+', vertical: '|' };
  const label = ' WOML WORKFLOW ';
  const top = `${corners.topLeft}${horizontal}${label}${horizontal.repeat(Math.max(0, inner - visibleLength(label) - 1))}${corners.topRight}`;
  const title = alignEnds(workflow.name ?? workflow.id, workflow.version === undefined ? '' : `v${workflow.version}`, inner - 2);
  const lines = [
    paint(top, 'cyan', options.color),
    `${paint(corners.vertical, 'cyan', options.color)} ${paint(title, 'bold', options.color)} ${paint(corners.vertical, 'cyan', options.color)}`,
  ];
  for (const description of wrap(workflow.description ?? '', inner - 2)) {
    lines.push(`${paint(corners.vertical, 'cyan', options.color)} ${padRight(description, inner - 2)} ${paint(corners.vertical, 'cyan', options.color)}`);
  }
  const bottom = `${corners.bottomLeft}${horizontal.repeat(inner)}${corners.bottomRight}`;
  lines.push(paint(bottom, 'cyan', options.color));
  return lines;
}

function triggerTitle(trigger: TriggerPresentationV1): string {
  const labels: Record<TriggerPresentationV1['type'], string> = {
    manual: 'Manual', webhook: 'Webhook', slack: 'Slack', telegram: 'Telegram', schedule: 'Schedule',
    interval: 'Interval', event: 'Event',
  };
  return trigger.label ?? labels[trigger.type];
}

function triggerDetails(
  trigger: TriggerPresentationV1,
  options: ResolvedRenderOptions
): string[] {
  if (trigger.type === 'manual') return [options.manualInstruction];
  if (trigger.type === 'webhook') return [
    `${trigger.method ?? 'POST'}  ${trigger.url ?? 'Address unavailable'}`,
    ...(trigger.example === undefined ? [] : ['', 'Try it', ...sanitizeTerminalText(trigger.example).split('\n')]),
  ];
  if (trigger.type === 'event') return [
    `Event  ${trigger.event ?? trigger.id}`,
    ...(trigger.url === undefined ? [] : [`POST   ${trigger.url}`]),
    ...(trigger.example === undefined ? [] : ['', 'Try it', ...sanitizeTerminalText(trigger.example).split('\n')]),
  ];
  if (trigger.type === 'slack') return [
    `Workspace  ${trigger.workspace ?? 'configured workspace'}`,
    ...(trigger.scope === undefined ? [] : [`Scope      ${trigger.scope}`]),
  ];
  if (trigger.type === 'telegram') return [
    'Send a message to the configured Telegram bot.',
    ...(trigger.scope === undefined ? [] : [`Events     ${trigger.scope}`]),
  ];
  if (trigger.type === 'schedule') return [
    `Schedule   ${trigger.schedule ?? 'configured schedule'}`,
    `Timezone   ${trigger.timezone ?? 'UTC'}`,
    ...(trigger.nextDueAt === undefined ? [] : [`Next       ${trigger.nextDueAt}`]),
  ];
  return [
    `Every      ${trigger.interval ?? 'configured interval'}`,
    ...(trigger.nextDueAt === undefined ? [] : [`Next       ${trigger.nextDueAt}`]),
  ];
}

export function renderWorkflowStartup(
  workflow: WorkflowPresentationV1,
  renderOptions: PresentationRenderOptions = {}
): string {
  const options = resolveOptions(renderOptions);
  if (options.format === 'json') return `${JSON.stringify(workflow)}\n`;
  const safe = sanitizePresentation({
    profile: 'woml.run-presentation/v1', workflow, runId: 'presentation',
    trigger: { id: 'presentation', type: 'manual' }, status: 'queued',
    admittedAt: '1970-01-01T00:00:00.000Z', steps: [],
    summary: { succeeded: 0, failed: 0, skipped: 0, cancelled: 0, total: 0 },
    lifecycle: [], warnings: [],
  }).workflow;
  const lines = [...workflowBox(safe, options), '', paint(safe.triggers.length === 1 ? 'TRIGGER' : 'TRIGGERS', 'bold', options.color), ''];
  for (const trigger of safe.triggers) {
    lines.push(`  ${paint(options.unicode ? '●' : '*', 'blue', options.color)} ${paint(triggerTitle(trigger), 'bold', options.color)}`);
    for (const detail of triggerDetails(trigger, options)) lines.push(detail.length === 0 ? '' : `    ${detail}`);
    if (trigger.warning !== undefined) lines.push(`    ${paint(`Warning: ${trigger.warning}`, 'yellow', options.color)}`);
    lines.push('');
  }
  lines.push(`  ${paint(options.unicode ? '●' : '*', 'green', options.color)} ${paint('Ready', 'green', options.color)} · Press Ctrl+C to stop`);
  return `${lines.join('\n')}\n`;
}

function compactValue(value: JsonValue): string {
  if (value === null || typeof value === 'boolean' || typeof value === 'number' || typeof value === 'string') {
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) return `[${value.map(item => compactValue(item)).join(', ')}]`;
  return `{ ${Object.entries(value).map(([key, item]) => {
    const label = /^[A-Za-z_$][\w$]*$/u.test(key) ? key : JSON.stringify(key);
    return `${label}: ${compactValue(item)}`;
  }).join(', ')} }`;
}

function valueLines(value: JsonValue, width: number): string[] {
  const compact = compactValue(value);
  if (!compact.includes('\n') && visibleLength(compact) <= width) return [compact];
  return JSON.stringify(value, null, 2).split('\n').map(line => {
    const points = [...line];
    if (points.length <= width) return line;
    if (width <= 1) return '…';
    return `${points.slice(0, width - 1).join('')}…`;
  });
}

function stepLines(step: StepPresentationV1, index: number, runId: string, options: ResolvedRenderOptions): string[] {
  const lines: string[] = [];
  const number = String(index + 1).padStart(2, '0');
  const glyph = paint(statusGlyph(step.status, options.unicode), statusColor(step.status), options.color);
  const depth = '  '.repeat(Math.min(step.depth, 4));
  const prefix = `  ${number}  ${glyph}  ${depth}`;
  const label = step.name ?? step.id;
  const right = step.durationMs === undefined
    ? statusLabel(step.status)
    : duration(step.durationMs);
  lines.push(alignEnds(`${prefix}${paint(label, 'bold', options.color)}`, right, options.width));

  const contentIndent = `          ${depth}`;
  if (step.description !== undefined) {
    const description = step.kind === 'custom_step'
      ? `${stepKind(step.kind)} · ${step.description}`
      : step.description;
    for (const line of wrap(description, Math.max(12, options.width - visibleLength(contentIndent)))) {
      lines.push(`${contentIndent}${paint(line, 'dim', options.color)}`);
    }
  }
  const kindAlreadyShown = step.kind === 'custom_step' && step.description !== undefined;
  if (step.detail !== undefined || (!['step', 'script'].includes(step.kind) && !kindAlreadyShown)) {
    const detail = step.detail === undefined ? stepKind(step.kind) : `${stepKind(step.kind)} · ${step.detail}`;
    for (const line of wrap(detail, Math.max(12, options.width - visibleLength(contentIndent)))) {
      lines.push(`${contentIndent}${paint(line, 'dim', options.color)}`);
    }
  }
  if (step.result !== undefined) {
    const rendered = valueLines(step.result, Math.max(12, options.width - visibleLength(contentIndent) - 2));
    if (rendered.length === 1) lines.push(`${contentIndent}${paint(options.unicode ? '→' : '>', 'cyan', options.color)} ${rendered[0]}`);
    else {
      lines.push(`${contentIndent}${paint(options.unicode ? '→' : '>', 'cyan', options.color)} Result`);
      for (const line of rendered) lines.push(`${contentIndent}  ${line}`);
    }
    if (step.resultTruncated === true) {
      lines.push('');
      for (const line of wrap(`Complete result: ${options.fullResultCommand(runId)}`, Math.max(12, options.width - visibleLength(contentIndent)))) {
        lines.push(`${contentIndent}${paint(line, 'dim', options.color)}`);
      }
    }
  }
  if (step.failure !== undefined) {
    lines.push('', `${contentIndent}${paint('Error', 'red', options.color)}`);
    for (const line of wrap(step.failure.code, Math.max(12, options.width - visibleLength(contentIndent)))) {
      lines.push(`${contentIndent}${paint(line, 'red', options.color)}`);
    }
    for (const line of wrap(step.failure.message, Math.max(12, options.width - visibleLength(contentIndent)))) lines.push(`${contentIndent}${line}`);
  }
  if (step.attempts > 1) {
    const exhausted = step.status === 'failed' || step.status === 'timed_out' ? ' · Retry exhausted' : '';
    lines.push('', `${contentIndent}Attempts  ${step.attempts}${exhausted}`);
  }
  return lines;
}

function lifecycleLines(item: LifecyclePresentationV1, options: ResolvedRenderOptions): string[] {
  const glyph = paint(statusGlyph(item.status, options.unicode), statusColor(item.status), options.color);
  const lines = [alignEnds(`  ${glyph}  ${item.hook}`, duration(item.durationMs), options.width)];
  const detail = [item.provider, item.detail].filter((value): value is string => value !== undefined).join(' · ');
  if (detail.length > 0) {
    for (const line of wrap(detail, Math.max(12, options.width - 5))) lines.push(`     ${paint(line, 'dim', options.color)}`);
  }
  if (item.failure !== undefined) {
    for (const line of wrap(item.failure.code, Math.max(12, options.width - 5))) lines.push(`     ${paint(line, 'red', options.color)}`);
    for (const line of wrap(item.failure.message, Math.max(12, options.width - 5))) lines.push(`     ${line}`);
  }
  return lines;
}

function finalTitle(status: RunPresentationStatus): string {
  if (status === 'succeeded') return 'RUN COMPLETED';
  if (status === 'failed') return 'RUN FAILED';
  if (status === 'timed_out') return 'RUN TIMED OUT';
  if (status === 'cancelled') return 'RUN CANCELLED';
  if (status === 'finalizing') return 'RUN FINALIZING';
  return `RUN ${status.toUpperCase()}`;
}

export function renderRunPresentation(
  presentation: RunPresentationV1,
  renderOptions: PresentationRenderOptions = {}
): string {
  const options = resolveOptions(renderOptions);
  const safe = sanitizePresentation(presentation);
  if (options.format === 'json') return `${JSON.stringify(safe)}\n`;
  const lines = [
    alignEnds(`${paint('RUN', 'bold', options.color)}  ${paint(safe.runId, 'cyan', options.color)}`, formatDate(safe.admittedAt, options), options.width),
    '',
  ];
  safe.steps.forEach((step, index) => {
    lines.push(...stepLines(step, index, safe.runId, options), '');
  });
  const terminal = ['succeeded', 'failed', 'cancelled', 'timed_out'].includes(safe.status);
  lines.push('', paint(terminal ? 'STEPS COMPLETED' : 'STEP PROGRESS', 'bold', options.color), '');
  lines.push(`  Duration    ${duration(safe.steps.reduce((sum, step) => sum + (step.durationMs ?? 0), 0))}`);
  const summaryParts = [`${safe.summary.succeeded} succeeded`, `${safe.summary.failed} failed`, `${safe.summary.skipped} skipped`];
  if (safe.summary.cancelled > 0) summaryParts.push(`${safe.summary.cancelled} cancelled`);
  const summaryLines = wrap(summaryParts.join(' · '), Math.max(12, options.width - 14));
  lines.push(`  Steps       ${summaryLines.shift() ?? ''}`);
  for (const line of summaryLines) lines.push(`              ${line}`);

  if (safe.lifecycle.length > 0 || safe.warnings.length > 0) {
    lines.push('', '', paint('LIFECYCLE', 'bold', options.color), '');
    for (const item of safe.lifecycle) lines.push(...lifecycleLines(item, options), '');
    for (const warning of safe.warnings) {
      lines.push(`  ${paint(options.unicode ? '!' : 'WARN', 'yellow', options.color)}  ${paint(warning.code, 'yellow', options.color)}`);
      lines.push(`     ${warning.message}`, '');
    }
  }

  const glyph = paint(statusGlyph(safe.status, options.unicode), statusColor(safe.status), options.color);
  lines.push('', alignEnds(`${glyph} ${paint(finalTitle(safe.status), statusColor(safe.status), options.color)}`, duration(safe.durationMs), options.width));
  if (safe.result !== undefined) {
    lines.push('', '  Final result');
    for (const line of valueLines(safe.result, Math.max(12, options.width - 2))) lines.push(`  ${line}`);
    if (safe.resultTruncated === true) {
      lines.push('');
      for (const line of wrap(`Complete result: ${options.fullResultCommand(safe.runId)}`, Math.max(12, options.width - 2))) {
        lines.push(`  ${paint(line, 'dim', options.color)}`);
      }
    }
  }
  if (safe.failure !== undefined) {
    lines.push('');
    for (const line of wrap(safe.failure.code, Math.max(12, options.width - 2))) lines.push(`  ${paint(line, 'red', options.color)}`);
    for (const line of wrap(safe.failure.message, Math.max(12, options.width - 2))) lines.push(`  ${line}`);
  }
  return `${lines.join('\n').replace(/\n{4,}/gu, '\n\n\n')}\n`;
}

export function renderReadyPrompt(
  workflow: WorkflowPresentationV1,
  renderOptions: PresentationRenderOptions = {}
): string {
  const options = resolveOptions(renderOptions);
  const hasManual = workflow.triggers.some(trigger => trigger.type === 'manual');
  if (!hasManual || options.format === 'json') return '';
  return `${paint(options.unicode ? '●' : '*', 'green', options.color)} ${paint('Ready', 'green', options.color)} · Press Enter to run again\n`;
}

export interface ManualTargetPresentation {
  readonly workflowId: string;
  readonly triggerId: string;
}

export function renderManualTargetSelection(
  targets: readonly ManualTargetPresentation[],
  renderOptions: PresentationRenderOptions = {}
): string {
  if (targets.length === 0) return '';
  const options = resolveOptions(renderOptions);
  if (options.format === 'json') return '';
  if (targets.length === 1) {
    return `${paint(options.unicode ? '●' : '*', 'green', options.color)} ${paint('Ready', 'green', options.color)} · Press Enter to run\n`;
  }
  const lines = [paint('MANUAL TRIGGERS', 'bold', options.color), ''];
  targets.forEach((target, index) => {
    lines.push(
      `  ${paint(String(index + 1).padStart(2, ' '), 'cyan', options.color)}  ${sanitizeTerminalText(target.workflowId)} / ${sanitizeTerminalText(target.triggerId)}`
    );
  });
  lines.push('', '  Type a number and press Enter to run');
  return `${lines.join('\n')}\n`;
}

export interface RunAdmissionPresentation {
  readonly runId: string;
  readonly admittedAt: string;
  readonly workflowId: string;
  readonly triggerId: string;
  readonly triggerType: TriggerPresentationV1['type'];
}

/** Render the immediate receipt shown before durable execution settles. */
export function renderRunAdmission(
  admission: RunAdmissionPresentation,
  renderOptions: PresentationRenderOptions = {}
): string {
  const options = resolveOptions(renderOptions);
  const safeRunId = sanitizeTerminalText(admission.runId);
  const safeWorkflowId = sanitizeTerminalText(admission.workflowId);
  const safeTriggerId = sanitizeTerminalText(admission.triggerId);
  const safeDate = formatDate(admission.admittedAt, options);
  if (options.format === 'json') return '';
  const title = alignEnds(
    `${paint('RUN', 'bold', options.color)}  ${paint(safeRunId, 'cyan', options.color)}`,
    safeDate,
    options.width
  );
  const trigger = `${admission.triggerType} · ${safeTriggerId}`;
  return `\n${title}\n  ${paint(options.unicode ? '●' : '*', 'blue', options.color)} ${paint('Accepted', 'blue', options.color)} · ${trigger} · ${safeWorkflowId}\n`;
}

export function renderRunNotice(
  runId: string,
  status: 'queued' | 'waiting' | 'retrying' | 'finalizing',
  message: string,
  renderOptions: PresentationRenderOptions = {}
): string {
  const options = resolveOptions(renderOptions);
  const safeRunId = sanitizeTerminalText(runId);
  const safeMessage = sanitizePresentationDiagnostic(message);
  if (options.format === 'json') return '';
  const glyph = status === 'finalizing' ? (options.unicode ? '◇' : '*') : (options.unicode ? '○' : '*');
  return `  ${paint(glyph, 'yellow', options.color)} ${paint(safeRunId, 'cyan', options.color)} · ${paint(status, 'yellow', options.color)} · ${safeMessage}\n`;
}

export function renderPresentationWarning(
  code: string,
  message: string,
  renderOptions: PresentationRenderOptions = {}
): string {
  const options = resolveOptions(renderOptions);
  const safeCode = sanitizeTerminalText(code).replaceAll('\n', ' ');
  const safeMessage = sanitizePresentationDiagnostic(message);
  if (options.format === 'json') return '';
  return `${paint(options.unicode ? '!' : 'WARN', 'yellow', options.color)} ${paint(safeCode, 'yellow', options.color)} · ${safeMessage}\n`;
}
