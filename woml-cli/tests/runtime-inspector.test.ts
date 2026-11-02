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
import { openTelemetryInspectorView } from '../src/opentelemetry-runtime';
import { createInspectorAnalytics } from '../src/runtime-inspector/analytics';
import { fit, safeText } from '../src/runtime-inspector/terminal';

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
    expect(parseInspectArguments(['inspect', 'orders.woml', '--state', 'custom.sqlite', '--no-color'])).toMatchObject({ workflowFile: expect.stringContaining('orders.woml'), statePath: expect.stringContaining('custom.sqlite'), color: false });
    for (const args of [['inspect', 'one.woml', 'two.woml'], ['inspect', 'orders'], ['inspect', '--state'], ['inspect', '--no-color', '--no-color']]) expect(() => parseInspectArguments(args)).toThrow('Usage');
  });

  test('matches the reference dashboard hierarchy, green header and gray selection', () => {
    const colored = renderInspectorFrame(snapshot, { ...baseState, view: 'runs' }, 140, 34, true);
    const plain = safeText(colored.replaceAll('\r\n', '\n')).replaceAll(' ', '');
    for (const marker of ['WOMLINSPECT', 'Active', 'Queue', 'Waits', 'Failed', 'runtime:healthy', 'Uptime:02:03:04', 'View[Runs▾]',
      'STATUS', 'WORKFLOW', 'RUNID', 'WORK', 'DURATION', 'Selected:run_running', 'Execution:order-processing→chargeCustomer[running]', '→Analytics', '←BackToList']) expect(plain).toContain(marker);
    expect(colored).toContain('\x1b[30;48;2;0;204;51m');
    expect(colored).toContain('\x1b[30;48;2;166;166;166m');
    expect(colored).toContain('\x1b[K');
  });

  test('renders real analytics with all reference sections and honest partial coverage', async () => {
    const provider = createInspectorAnalytics({ workflows: [{ workflowId: 'order-processing', name: 'Order processing', file: 'orders.woml', triggerTypes: ['manual'] }],
      listRuns: () => ({ profile: 'woml.run-list/v1', runs: [] }), inspectRun: () => { throw new Error('not called'); } });
    const analytics = await provider.get('order-processing');
    const frame = renderInspectorFrame(snapshot, { ...baseState, analyticsWorkflowId: 'order-processing', analytics }, 140, 44, false);
    for (const marker of ['WORKFLOW / ANALYTICS', 'Last 24 hours · UTC', 'Order processing', 'orders.woml', 'Finished runs', 'Success rate', 'Avg. duration', 'Retried runs', 'RUN ACTIVITY', '4-hour intervals', 'STEP PERFORMANCE', 'RECENT FAILURES', 'Median', 'Longest', 'Back To List', 'latest 0 finished runs']) expect(frame).toContain(marker);
    expect(frame).not.toContain('Sample data');
    expect(frame).not.toContain('NaN');
    expect(frame).not.toContain('100.0%');
  });

  test('keeps every rendered line within the physical terminal and sanitizes hostile workflow text', () => {
    const hostile = { ...snapshot, runs: snapshot.runs.map(run => ({ ...run, workflowId: '\x1b[31m工作流👩‍💻\x1b]52;c;PRIVATE_CLIPBOARD\x07' })) };
    for (const [width, height] of [[140, 34], [100, 24], [80, 24], [40, 12], [20, 8], [1, 1]]) {
      for (const color of [true, false]) {
        const frame = renderInspectorFrame(hostile, { ...baseState, view: 'runs' }, width!, height!, color);
        const lines = frame.split('\r\n');
        expect(lines.length).toBeLessThanOrEqual(height!);
        for (const line of lines) expect(Bun.stringWidth(line)).toBeLessThanOrEqual(width!);
        expect(frame).not.toContain('PRIVATE_CLIPBOARD');
        if (!color) expect(frame).not.toMatch(/\x1b\[[0-9;]*m/);
      }
    }
    expect(Bun.stringWidth(fit('雪👩‍💻café', 6))).toBe(6);
  });

  test('keeps selection visible when scrolling and filters the file workflow by exact ID', () => {
    const many = { ...snapshot, runs: Array.from({ length: 100 }, (_, i) => ({ ...snapshot.runs[0]!, runId: `run_${i}` })) };
    const frame = renderInspectorFrame(many, { ...baseState, view: 'runs', selected: 99 }, 100, 24, false);
    expect(frame).toContain('Selected: run_99');
    expect(frame).not.toContain('run_0 ');
    expect(frame).toContain('┃');
    const scoped = renderInspectorFrame(snapshot, { ...baseState, view: 'runs', workflowId: 'order-processing' }, 140, 30, false);
    expect(scoped).not.toContain('inventory-sync');
    expect(scoped).toContain('Runs: 3');
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

  test('renders bounded OpenTelemetry exporter health in the runtime view', () => {
    const frame = renderInspectorFrame(
      {
        ...snapshot,
        openTelemetryView: openTelemetryInspectorView({
          profile: 'woml.runtime-health/v2',
          openTelemetry: {
            profile: 'woml.opentelemetry-exporter-health/v2',
            state: 'degraded',
            enabled: true,
            protocol: 'http/protobuf',
            signals: ['traces', 'metrics'],
            queue: { size: 96, capacity: 128, utilization: 0.75 },
            exports: 12,
            failures: 2,
            timeouts: 1,
            dropped: 3,
            flushes: 1,
            consecutiveFailures: 2,
            circuit: 'open',
            lastSuccessAt: '2026-10-23T11:59:00.000Z',
            retryAt: '2026-10-23T12:00:05.000Z',
            lastErrorCode: 'WOML_OTEL_EXPORT_FAILED',
          },
        }),
      },
      { ...baseState, view: 'runtime' },
      120,
      30,
      false
    );
    expect(frame).toContain('OPENTELEMETRY EXPORT');
    expect(frame).toContain('Queue: 96 / 128 (75%)');
    expect(frame).toContain('Exports: 12  Failures: 2  Timeouts: 1  Dropped: 3');
    expect(frame).not.toContain('authorization');
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

  test('opens analytics lazily, scrolls, returns to selection, and keeps view selection accessible', async () => {
    const terminal = new VirtualTerminal();
    const provider = createInspectorAnalytics({ workflows: [{ workflowId: 'order-processing', triggerTypes: ['manual'] }],
      listRuns: () => ({ profile: 'woml.run-list/v1', runs: [] }), inspectRun: () => { throw new Error('not called'); } });
    const analytics = await provider.get('order-processing');
    let reads = 0;
    const base = inspectorFetch();
    const fetcher = (async (input: string | URL | Request, init?: RequestInit) => {
      if (String(input).includes('/v1/inspector/analytics')) { reads++; expect(new Headers(init?.headers).get('authorization')).toBe(`Bearer ${descriptor.capability}`); return Response.json(analytics); }
      return base(input, init);
    }) as typeof fetch;
    const running = runRuntimeInspector({ args: { statePath: '/tmp/inspect/state.sqlite', color: false }, terminal, fetcher, readDescriptor: async () => descriptor });
    await eventually(() => terminal.rawModes.includes(true));
    expect(reads).toBe(0);
    terminal.input('\x1b[C');
    await eventually(() => terminal.writes.some(write => write.includes('Finished runs')));
    terminal.input('\x1b[6~');
    await eventually(() => terminal.writes.at(-1)?.includes('Step/retry/failure details') === true);
    expect(reads).toBe(1);
    terminal.input('\x1b[D');
    await eventually(() => terminal.writes.at(-1)?.includes('Selected: run_running') === true);
    terminal.input('v');
    await eventually(() => terminal.writes.at(-1)?.includes('7 Runtime') === true);
    terminal.input('7');
    await eventually(() => terminal.writes.at(-1)?.includes('COMPONENT') === true);
    terminal.input('q');
    expect(await running).toBe(0);
  });

  test('shows unavailable analytics without crashing and permits Ctrl+C while searching', async () => {
    const terminal = new VirtualTerminal();
    const running = runRuntimeInspector({ args: { statePath: '/tmp/inspect/state.sqlite', color: false }, terminal, fetcher: inspectorFetch(), readDescriptor: async () => descriptor });
    await eventually(() => terminal.rawModes.includes(true));
    terminal.input('\x1b[C');
    await eventually(() => terminal.writes.some(write => write.includes('Analytics unavailable')));
    terminal.input('\x1b[D'); terminal.input('/'); terminal.input('\x03');
    expect(await running).toBe(0);
    expect(terminal.rawModes).toEqual([true, false]);
  });

  test('does not apply a late analytics response after returning to the list', async () => {
    const terminal = new VirtualTerminal();
    let release!: (response: Response) => void;
    let requested = false;
    const base = inspectorFetch();
    const fetcher = (async (input: string | URL | Request, init?: RequestInit) => {
      if (String(input).includes('/v1/inspector/analytics')) {
        requested = true;
        return await new Promise<Response>(resolve => { release = resolve; });
      }
      return base(input, init);
    }) as typeof fetch;
    const running = runRuntimeInspector({ args: { statePath: '/tmp/inspect/state.sqlite', color: false }, terminal, fetcher, readDescriptor: async () => descriptor });
    await eventually(() => terminal.rawModes.includes(true));
    terminal.input('\x1b[C'); await eventually(() => requested);
    terminal.input('\x1b[D'); release(Response.json({ invalid: true }));
    await eventually(() => terminal.writes.at(-1)?.includes('Selected: run_running') === true);
    expect(terminal.writes.at(-1)).not.toContain('Analytics unavailable');
    terminal.input('q'); expect(await running).toBe(0);
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
