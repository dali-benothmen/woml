import { describe, expect, test } from 'bun:test';

import {
  parseInspectArguments,
  renderInspectorFrame,
  runRuntimeInspector,
  type InspectorRenderState,
  type InspectorSnapshotV1,
  type InspectorTerminal,
} from '../src/runtime-inspector';
import type { RuntimeDescriptorV1 } from '../src/runtime-control';

const snapshot: InspectorSnapshotV1 = {
  profile: 'woml.runtime-operations-snapshot/v1',
  runtimeInstanceId: 'runtime_inspect',
  sequence: 7,
  capturedAt: '2026-08-12T12:00:00.000Z',
  lifecycle: 'ready',
  ready: true,
  uptimeMs: 7_384_000,
  workflows: [
    {
      workflowId: 'order-processing',
      definitionHash: `sha256:${'a'.repeat(64)}`,
      active: 1,
      queued: 1,
      waiting: 1,
      failed: 1,
      triggerTypes: ['webhook', 'slack'],
    },
  ],
  runs: [
    {
      runId: 'run_running',
      workflowId: 'order-processing',
      status: 'running',
      durationMs: 1800,
      currentNodeId: 'chargeCustomer',
      forEach: [{
        runId: 'run_running',
        forEachId: 'organize',
        status: 'running',
        total: 42,
        succeeded: 18,
        failed: 0,
        skipped: 0,
        active: 4,
        pending: 20,
        concurrency: 4,
      }],
    },
    {
      runId: 'run_waiting',
      workflowId: 'order-processing',
      status: 'waiting',
      durationMs: 720_000,
      currentNodeId: 'managerApproval',
    },
    {
      runId: 'run_retrying',
      workflowId: 'inventory-sync',
      status: 'retrying',
      durationMs: 4200,
      currentNodeId: 'updateInventory',
    },
    {
      runId: 'run_failed',
      workflowId: 'order-processing',
      status: 'failed',
      durationMs: 643,
    },
  ],
  components: [
    { name: 'sqlite', kind: 'store', status: 'ready' },
    { name: 'slack', kind: 'provider', status: 'degraded', code: 'WOML_PROVIDER_RECONNECTING' },
  ],
  alerts: [
    {
      at: '2026-08-12T11:59:59.000Z',
      level: 'warn',
      code: 'WOML_PROVIDER_RECONNECTING',
      message: 'Slack provider is reconnecting.',
    },
  ],
};

const baseState: InspectorRenderState = {
  view: 'overview',
  selected: 0,
  filter: '',
  expanded: false,
  showHelp: false,
  stale: false,
  recentEvents: [],
};

class VirtualTerminal implements InspectorTerminal {
  readonly isTTY: boolean;
  columns = 100;
  rows = 24;
  readonly writes: string[] = [];
  readonly rawModes: boolean[] = [];
  #input?: (text: string) => void;
  #resize?: () => void;

  constructor(isTTY = true) {
    this.isTTY = isTTY;
  }

  write(text: string): void { this.writes.push(text); }
  setRawMode(enabled: boolean): void { this.rawModes.push(enabled); }
  onInput(listener: (text: string) => void): () => void {
    this.#input = listener;
    return () => { this.#input = undefined; };
  }
  onResize(listener: () => void): () => void {
    this.#resize = listener;
    return () => { this.#resize = undefined; };
  }
  input(text: string): void { this.#input?.(text); }
  resize(columns: number, rows: number): void {
    this.columns = columns;
    this.rows = rows;
    this.#resize?.();
  }
}

const descriptor: RuntimeDescriptorV1 = {
  profile: 'woml.runtime-descriptor/v1',
  runtimeInstanceId: 'runtime_inspect',
  deploymentId: 'deployment_inspect',
  pid: process.pid,
  adminUrl: 'http://127.0.0.1:31234',
  capability: 'a'.repeat(43),
  createdAt: new Date(Date.now() - 1000).toISOString(),
  expiresAt: new Date(Date.now() + 60_000).toISOString(),
};

function inspectorFetch(cancellations: string[] = []): typeof fetch {
  return (async (input: string | URL | Request, init?: RequestInit) => {
    const url = String(input);
    if (url.endsWith('/v1/snapshot')) return Response.json(snapshot);
    if (url.includes('/v1/stream')) {
      return new Response(new ReadableStream({ start() {} }), {
        headers: { 'content-type': 'text/event-stream' },
      });
    }
    if (url.endsWith('/v1/control')) {
      const body = JSON.parse(String(init?.body)) as Record<string, string>;
      cancellations.push(body.subjectId!);
      return Response.json({
        profile: 'woml.runtime-admin-http/v1',
        kind: 'response',
        requestId: body.requestId,
        status: 'accepted',
      });
    }
    return Response.json({ error: { code: 'NOT_FOUND' } }, { status: 404 });
  }) as typeof fetch;
}

async function eventually(check: () => boolean): Promise<void> {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (check()) return;
    await Bun.sleep(5);
  }
  throw new Error('Timed out waiting for the virtual terminal.');
}

describe('WOML terminal inspector', () => {
  test('uses the final inspect command and honors the conventional no-color switch', () => {
    expect(parseInspectArguments(['inspect'])).toMatchObject({
      color: !('NO_COLOR' in process.env),
    });
    expect(parseInspectArguments(['inspect', '--no-color']).color).toBe(false);
    expect(() => parseInspectArguments(['top'])).toThrow('Usage: woml inspect');
  });

  test('renders every operations view in narrow and normal terminals', () => {
    const expected: Record<string, string> = {
      overview: 'order-processing',
      runs: 'organize 18/42',
      triggers: 'webhook, slack',
      approvals: 'run_waiting',
      queues: 'run_retrying',
      failures: 'WOML_PROVIDER_RECONNECTING',
      runtime: 'sqlite',
    };
    for (const [view, marker] of Object.entries(expected)) {
      const frame = renderInspectorFrame(
        snapshot,
        { ...baseState, view: view as InspectorRenderState['view'] },
        view === 'queues' ? 40 : 100,
        24,
        false
      );
      expect(frame).toContain('WOML INSPECT');
      expect(frame).toContain(marker);
    }
  });

  test('shows concise loop work and bounded detail for the selected run', () => {
    const compact = renderInspectorFrame(snapshot, {
      ...baseState,
      view: 'runs',
    }, 100, 24, false);
    expect(compact).toContain('organize 18/42');

    const expanded = renderInspectorFrame(snapshot, {
      ...baseState,
      view: 'runs',
      expanded: true,
    }, 100, 24, false);
    expect(expanded).toContain(
      'For each organize: 18/42 completed · 4 active · 20 pending · concurrency 4'
    );
  });

  test('uses htop-style status colors while keeping a completely color-free mode', () => {
    const colored = renderInspectorFrame(snapshot, { ...baseState, view: 'runs', selected: 1 }, 100, 24, true);
    expect(colored).toContain('\u001b[1;32m');
    expect(colored).toContain('\u001b[1;34m');
    expect(colored).toContain('\u001b[1;33m');
    expect(colored).toContain('\u001b[1;31m');

    const plain = renderInspectorFrame(snapshot, { ...baseState, view: 'runs' }, 100, 24, false);
    expect(plain).not.toMatch(/\u001b\[(?:1;)?3[0-7]m/);
    expect(plain).toContain('running');
    expect(plain).toContain('waiting');
  });

  test('navigates, filters, resizes, confirms cancellation, and restores the terminal', async () => {
    const terminal = new VirtualTerminal();
    const cancellations: string[] = [];
    const running = runRuntimeInspector({
      args: { statePath: '/tmp/pro6/state.sqlite', color: true },
      terminal,
      fetcher: inspectorFetch(cancellations),
      readDescriptor: async () => descriptor,
    });
    await eventually(() => terminal.rawModes.includes(true));
    terminal.input('\u001b[B');
    terminal.input('/');
    terminal.input('order');
    terminal.input('\r');
    terminal.resize(70, 18);
    terminal.input('2');
    terminal.input('\u001b[A');
    terminal.input('c');
    await eventually(() => terminal.writes.some(write => write.includes('Press y to confirm')));
    terminal.input('y');
    await eventually(() => cancellations.length === 1);
    terminal.input('q');
    expect(await running).toBe(0);
    expect(cancellations).toEqual(['run_running']);
    expect(terminal.rawModes).toEqual([true, false]);
    expect(terminal.writes.at(0)).toContain('\u001b[?1049h');
    expect(terminal.writes.at(-1)).toContain('\u001b[?1049l');
  });

  test('rejects non-TTY use with a scriptable alternative and does not connect', async () => {
    const terminal = new VirtualTerminal(false);
    let connected = false;
    const exitCode = await runRuntimeInspector({
      args: { statePath: '/tmp/pro6/state.sqlite', color: true },
      terminal,
      readDescriptor: async () => {
        connected = true;
        return descriptor;
      },
    });
    expect(exitCode).toBe(2);
    expect(connected).toBe(false);
    expect(terminal.writes.join('')).toContain('woml list --json');
  });

  test('marks a stream gap stale, resynchronizes, and reconnects with a fresh descriptor', async () => {
    const terminal = new VirtualTerminal();
    let snapshots = 0;
    let streams = 0;
    let descriptorReads = 0;
    const fetcher = (async (input: string | URL | Request) => {
      const url = String(input);
      if (url.endsWith('/v1/snapshot')) {
        snapshots += 1;
        return Response.json({ ...snapshot, sequence: 8 });
      }
      if (url.includes('/v1/stream')) {
        streams += 1;
        if (streams === 1) {
          const gap = {
            profile: 'woml.runtime-operations-stream/v1',
            runtimeInstanceId: 'runtime_inspect',
            sequence: 8,
            occurredAt: '2026-08-12T12:00:01.000Z',
            kind: 'alert',
            subject: {
              id: 'stream',
              status: 'resync_required',
              code: 'WOML_OBSERVABILITY_STREAM_GAP',
            },
          };
          return new Response(`event: operations\ndata: ${JSON.stringify(gap)}\n\n`, {
            headers: { 'content-type': 'text/event-stream' },
          });
        }
        return new Response(new ReadableStream({ start() {} }), {
          headers: { 'content-type': 'text/event-stream' },
        });
      }
      return Response.json({ error: { code: 'NOT_FOUND' } }, { status: 404 });
    }) as typeof fetch;
    const running = runRuntimeInspector({
      args: { statePath: '/tmp/pro6/state.sqlite', color: false },
      terminal,
      fetcher,
      readDescriptor: async () => {
        descriptorReads += 1;
        return descriptor;
      },
    });
    await eventually(() => streams >= 2 && snapshots >= 2);
    terminal.input('q');
    expect(await running).toBe(0);
    expect(descriptorReads).toBeGreaterThanOrEqual(3);
  });

  test('restores raw mode and the alternate screen after a renderer failure', async () => {
    const rawModes: boolean[] = [];
    const writes: string[] = [];
    let input: ((text: string) => void) | undefined;
    const terminal: InspectorTerminal = {
      isTTY: true,
      columns: 80,
      rows: 24,
      write(text) {
        writes.push(text);
        if (writes.length === 2) throw new Error('virtual renderer failed');
      },
      setRawMode(enabled) { rawModes.push(enabled); },
      onInput(listener) { input = listener; return () => { input = undefined; }; },
      onResize() { return () => {}; },
    };
    const exitCode = await runRuntimeInspector({
      args: { statePath: '/tmp/pro6/state.sqlite', color: false },
      terminal,
      fetcher: inspectorFetch(),
      readDescriptor: async () => descriptor,
    });
    expect(input).toBeUndefined();
    expect(exitCode).toBe(1);
    expect(rawModes).toEqual([true, false]);
    expect(writes.at(-1)).toContain('\u001b[?1049l');
  });

  test('bounds rows and ignores arbitrary payload fields from the operations response', () => {
    const many = {
      ...snapshot,
      runs: Array.from({ length: 5000 }, (_, index) => ({
        runId: `run_${index}_${'x'.repeat(500)}`,
        workflowId: `unicode_工作流_${index}`,
        status: 'running' as const,
        durationMs: index,
        payload: 'PRIVATE_PAYLOAD_MUST_NOT_RENDER',
      })),
    };
    const started = performance.now();
    const frame = renderInspectorFrame(many, { ...baseState, view: 'runs' }, 80, 24, false);
    expect(frame.split('\n').length).toBeLessThanOrEqual(24);
    expect(frame).not.toContain('PRIVATE_PAYLOAD_MUST_NOT_RENDER');
    expect(performance.now() - started).toBeLessThan(100);
  });
});
