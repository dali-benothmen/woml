// Only our own styling may contain terminal escapes. Never trust workflow text.
export function safeText(value: string): string {
  return value.replace(/\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)?/g, '')
    .replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, '')
    .replace(/[\u0000-\u001f\u007f-\u009f\u202a-\u202e\u2066-\u2069]/g, ' ');
}

export function fit(value: string, width: number, align: 'left' | 'right' = 'left'): string {
  width = Math.max(0, width);
  const clipped = Bun.sliceAnsi(safeText(value), 0, width, { ellipsis: '…' });
  const space = ' '.repeat(Math.max(0, width - Bun.stringWidth(clipped)));
  return align === 'right' ? space + clipped : clipped + space;
}

export const tones = {
  green: '1;32', blue: '1;34', yellow: '1;33', red: '1;31',
  muted: '2', white: '1;37', header: '30;48;2;0;204;51', selected: '30;48;2;166;166;166', button: '37;48;5;234',
} as const;
export type Tone = keyof typeof tones;
export const paint = (value: string, tone: Tone, enabled: boolean): string =>
  enabled ? `\x1b[${tones[tone]}m${value}\x1b[0m` : value;
export function statusTone(status: string): Tone {
  if (status === 'succeeded' || status === 'healthy') return 'green';
  if (status === 'running' || status === 'queued') return 'blue';
  if (status === 'waiting' || status === 'retrying') return 'yellow';
  if (status === 'failed' || status === 'timed_out') return 'red';
  return 'white';
}
export function elapsed(ms: number | undefined): string {
  if (ms === undefined || !Number.isFinite(ms)) return '—';
  if (ms < 1000) return `${Math.max(0, Math.round(ms))}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  if (ms < 3_600_000) return `${Math.floor(ms / 60_000)}m`;
  return `${(ms / 3_600_000).toFixed(1)}h`;
}
export function uptime(ms: number): string {
  const seconds = Math.max(0, Math.floor(ms / 1000));
  return [Math.floor(seconds / 3600), Math.floor(seconds / 60) % 60, seconds % 60]
    .map(value => String(value).padStart(2, '0')).join(':');
}

// CRLF also works when the terminal has disabled LF -> CRLF translation.
export function frame(lines: readonly string[], height: number, width: number): string {
  return '\x1b[H' + lines.slice(0, height).map(line => (Bun.stringWidth(line) > width ? Bun.sliceAnsi(line, 0, width) : line) + '\x1b[K').join('\r\n') + '\x1b[J';
}
