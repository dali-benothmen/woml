import { describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';

import {
  renderReadyPrompt,
  renderRunPresentation,
  renderWorkflowStartup,
  sanitizeTerminalText,
  stripAnsi,
  type RunPresentationV1,
} from '../src/terminal-presentation';

const fixtureRoot = resolve(import.meta.dir, 'fixtures/terminal-presentation');

async function success(): Promise<RunPresentationV1> {
  return await Bun.file(resolve(fixtureRoot, 'success.v1.json')).json();
}

describe('terminal presentation renderer', () => {
  test('renders the reviewed workflow startup fixture', async () => {
    const fixture = await success();
    const actual = renderWorkflowStartup(fixture.workflow, {
      format: 'plain',
      width: 72,
      unicode: true,
      locale: 'en-GB',
      timeZone: 'UTC',
    });
    expect(actual).toBe(
      await Bun.file(resolve(fixtureRoot, 'workflow-startup.72.txt')).text()
    );
  });

  test('renders the reviewed successful run fixture', async () => {
    const fixture = await success();
    const actual = renderRunPresentation(fixture, {
      format: 'plain',
      width: 72,
      unicode: true,
      locale: 'en-GB',
      timeZone: 'UTC',
    });
    expect(actual).toBe(
      await Bun.file(resolve(fixtureRoot, 'success.72.txt')).text()
    );
  });

  test('renders the reviewed wide activation instructions for every trigger', async () => {
    const workflow = await Bun.file(resolve(fixtureRoot, 'triggers.v1.json')).json();
    const actual = renderWorkflowStartup(workflow, {
      format: 'plain', width: 100, unicode: true, timeZone: 'UTC',
    });
    expect(actual).toBe(await Bun.file(resolve(fixtureRoot, 'triggers.100.txt')).text());
  });

  test('uses colors in a capable TTY while preserving the plain semantics', async () => {
    const fixture = await success();
    const plain = renderRunPresentation(fixture, { format: 'plain', width: 72, unicode: true, timeZone: 'UTC' });
    const colored = renderRunPresentation(fixture, {
      format: 'tty', color: 'auto', isTTY: true, width: 72, timeZone: 'UTC', environment: { TERM: 'xterm-256color' },
    });
    expect(colored).toContain('\u001b[');
    expect(stripAnsi(colored)).toBe(plain);
  });

  test('honors NO_COLOR, TERM=dumb, redirected output, and explicit color modes', async () => {
    const fixture = await success();
    const cases = [
      { format: 'tty' as const, isTTY: true, environment: { TERM: 'xterm', NO_COLOR: '' } },
      { format: 'tty' as const, isTTY: true, environment: { TERM: 'dumb' } },
      { format: 'tty' as const, isTTY: false, environment: { TERM: 'xterm' } },
      { format: 'tty' as const, isTTY: true, color: 'never' as const, environment: { TERM: 'xterm' } },
      { format: 'plain' as const, isTTY: true, color: 'always' as const, environment: { TERM: 'xterm' } },
    ];
    for (const options of cases) {
      expect(renderRunPresentation(fixture, { ...options, width: 72 })).not.toContain('\u001b[');
    }
    expect(renderRunPresentation(fixture, {
      format: 'tty', isTTY: false, color: 'always', environment: { TERM: 'xterm' }, width: 72,
    })).toContain('\u001b[');
  });

  test('produces sanitized deterministic machine output', async () => {
    const fixture = await success();
    const unsafe: RunPresentationV1 = {
      ...structuredClone(fixture),
      workflow: { ...structuredClone(fixture.workflow), name: '\u001b[2JStolen title' },
      result: {
        message: 'safe\u0000value',
        token: 'must-not-appear',
        nested: { authorization: 'Bearer hidden' },
      },
    };
    const text = renderRunPresentation(unsafe, { format: 'json' });
    expect(text).not.toContain('\u001b');
    expect(text).not.toContain('must-not-appear');
    expect(text).not.toContain('Bearer hidden');
    const parsed = JSON.parse(text);
    expect(parsed.workflow.name).toBe('Stolen title');
    expect(parsed.result).toEqual({
      message: 'safe value',
      token: '[redacted]',
      nested: { authorization: '[redacted]' },
    });
  });

  test('bounds deeply nested, wide, and long returned values', async () => {
    const fixture = await success();
    const unsafe: RunPresentationV1 = {
      ...structuredClone(fixture),
      result: {
        values: Array.from({ length: 100 }, (_, index) => index),
        long: 'x'.repeat(2000),
        deep: { a: { b: { c: { d: { e: { f: 'too deep' } } } } } },
      },
    };
    const parsed = JSON.parse(renderRunPresentation(unsafe, { format: 'json' }));
    expect(parsed.result.values).toHaveLength(21);
    expect(parsed.result.values.at(-1)).toBe('[80 more items]');
    expect(parsed.result.long.length).toBeLessThan(510);
    expect(JSON.stringify(parsed.result)).toContain('maximum depth reached');
  });

  test('adapts to narrow terminals and keeps every plain line bounded', async () => {
    const fixture = await success();
    const output = renderRunPresentation(fixture, {
      format: 'plain', width: 36, unicode: false, timeZone: 'UTC',
    });
    expect(output).toContain('RUN  run_8f21c4');
    expect(output).toContain('RUN COMPLETED');
    expect(output).toBe(await Bun.file(resolve(fixtureRoot, 'success.36.ascii.txt')).text());
    for (const line of output.trimEnd().split('\n')) expect([...line].length).toBeLessThanOrEqual(36);
  });

  test('renders failures, retries, skipped work, lifecycle, and final failure separately', async () => {
    const fixture = await Bun.file(resolve(fixtureRoot, 'failure.v1.json')).json() as RunPresentationV1;
    const output = renderRunPresentation(fixture, { format: 'plain', width: 72, timeZone: 'UTC' });
    expect(output).toContain('WOML_HTTP_REQUEST_FAILED');
    expect(output).toContain('Attempts  3 · Retry exhausted');
    expect(output).toContain('Build confirmation');
    expect(output).toContain('Skipped');
    expect(output.indexOf('STEPS COMPLETED')).toBeLessThan(output.indexOf('LIFECYCLE'));
    expect(output.indexOf('LIFECYCLE')).toBeLessThan(output.indexOf('RUN FAILED'));
  });

  test('organizes nested automation while a durable run is still waiting', async () => {
    const fixture = await Bun.file(
      resolve(fixtureRoot, 'complex-control-flow.v1.json')
    ).json() as RunPresentationV1;
    const output = renderRunPresentation(fixture, {
      format: 'plain', width: 88, unicode: true, timeZone: 'UTC',
    });
    expect(output).toContain('STEP PROGRESS');
    expect(output).not.toContain('STEPS COMPLETED');
    expect(output).toContain('Switch · Selected "express".');
    expect(output).toContain('Parallel · 2 children · up to 2 at once');
    expect(output).toContain('Fork · 3 branches · join slack, email');
    expect(output).toContain('Branch · Runs independently');
    expect(output).toContain('Workflow call · Waiting for calculate-risk · run_call_42');
    expect(output).toContain('Workflow start · Started audit-trail · run_call_43 · detached');
    expect(output).toContain('Approval · Waiting for decision until');
    expect(output).toContain('Record analytics');
    expect(output).toContain('Retrying');
    expect(output).toContain('Confirm order');
    expect(output).toContain('Queued');
    expect(output).toContain('6 succeeded · 0 failed · 0 skipped');
    expect(output.indexOf('Send Slack update')).toBeLessThan(
      output.indexOf('Record analytics')
    );
    expect(output.indexOf('STEP PROGRESS')).toBeLessThan(output.indexOf('LIFECYCLE'));
    expect(output).toContain('RUN WAITING');

    const colored = renderRunPresentation(fixture, {
      format: 'tty', color: 'always', isTTY: true, width: 88, unicode: true,
      timeZone: 'UTC',
    });
    expect(colored).toContain('\u001b[33m');
    expect(stripAnsi(colored)).toBe(output);
  });

  test('renders concise loop progress while preserving bounded per-item JSON detail', async () => {
    const fixture = await success();
    const loop: RunPresentationV1 = {
      ...structuredClone(fixture),
      status: 'running',
      completedAt: undefined,
      steps: [{
        id: 'organize',
        name: 'Organize files',
        description: 'Classify every discovered file.',
        kind: 'for_each',
        status: 'running',
        depth: 0,
        attempts: 1,
        detail: '18/42 completed · 4 active · concurrency 4',
        forEach: {
          total: 42,
          succeeded: 18,
          failed: 0,
          skipped: 0,
          active: 4,
          pending: 20,
          concurrency: 4,
          iterations: [
            { index: 0, itemNumber: 1, status: 'succeeded' },
            { index: 18, itemNumber: 19, status: 'running' },
          ],
          iterationsTruncated: true,
        },
      }],
      summary: { succeeded: 0, failed: 0, skipped: 0, cancelled: 0, total: 1 },
    };

    const human = renderRunPresentation(loop, {
      format: 'plain', width: 88, unicode: true, timeZone: 'UTC',
    });
    expect(human).toContain('Organize files');
    expect(human).toContain('For each · 18/42 completed · 4 active · concurrency 4');
    expect(human).not.toContain('itemNumber');

    const machine = JSON.parse(renderRunPresentation(loop, { format: 'json' }));
    expect(machine.steps[0].forEach).toMatchObject({
      total: 42,
      succeeded: 18,
      active: 4,
      pending: 20,
      iterationsTruncated: true,
    });
    expect(machine.steps[0].forEach.iterations).toHaveLength(2);
  });

  test('shows a ready prompt only for manual workflows and never in JSON', async () => {
    const fixture = await success();
    expect(renderReadyPrompt(fixture.workflow, { format: 'plain', unicode: true })).toBe('● Ready · Press Enter to run again\n');
    expect(renderReadyPrompt(fixture.workflow, { format: 'json' })).toBe('');
    expect(renderReadyPrompt({ ...fixture.workflow, triggers: [{ id: 'orders', type: 'webhook' }] }, { format: 'plain' })).toBe('');
  });

  test('does not mutate the frozen presentation input', async () => {
    const fixture = await success();
    const before = JSON.stringify(fixture);
    renderRunPresentation(fixture, { format: 'tty', color: 'always' });
    expect(JSON.stringify(fixture)).toBe(before);
  });

  test('sanitizes standalone terminal text', () => {
    expect(sanitizeTerminalText('hello\u001b[2J\u0000world\r\nnext')).toBe('hello world\nnext');
  });
});
