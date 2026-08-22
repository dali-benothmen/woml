import { describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';

import {
  decodeRunPresentationV1,
  renderRunPresentation,
  RunPresentationDecodeError,
  type RunPresentationV1,
} from '../src/terminal-presentation';

async function loopPresentation(): Promise<RunPresentationV1> {
  const fixture = await Bun.file(resolve(
    import.meta.dir,
    'fixtures/terminal-presentation/success.v1.json'
  )).json() as RunPresentationV1;
  const loop = {
    id: 'processItems',
    name: 'Process items',
    kind: 'for_each' as const,
    status: 'failed' as const,
    depth: 0,
    attempts: 1,
    detail: 'Item 2 of 3 · index 1 · step "sendItem"',
    failure: {
      code: 'WOML_SCRIPT_THROWN',
      message: 'Request failed with Bearer secret-token and api_key=hidden-key',
    },
    forEach: {
      total: 3,
      succeeded: 1,
      failed: 1,
      skipped: 1,
      active: 0,
      pending: 0,
      concurrency: 2,
      iterations: [
        { index: 0, itemNumber: 1, status: 'succeeded' as const },
        {
          index: 1,
          itemNumber: 2,
          status: 'failed' as const,
          failedNodeId: 'sendItem',
          failure: {
            code: 'WOML_SCRIPT_THROWN',
            message: 'Slack token xoxb-sensitive must not leave diagnostics.',
          },
        },
        { index: 2, itemNumber: 3, status: 'skipped' as const },
      ],
    },
  };
  return {
    ...structuredClone(fixture),
    status: 'failed',
    steps: [loop],
    summary: { succeeded: 0, failed: 1, skipped: 0, cancelled: 0, total: 1 },
    failure: loop.failure,
  };
}

describe('for-each presentation hardening', () => {
  test('rejects raw item data, count drift, invalid item identity, and unbounded detail', async () => {
    const valid = await loopPresentation();
    expect(() => decodeRunPresentationV1(JSON.stringify(valid))).not.toThrow();

    const rawItem = structuredClone(valid) as any;
    rawItem.steps[0].forEach.iterations[0].item = { secret: 'must-not-pass' };
    expect(() => decodeRunPresentationV1(JSON.stringify(rawItem))).toThrow(
      RunPresentationDecodeError
    );

    const countDrift = structuredClone(valid) as any;
    countDrift.steps[0].forEach.pending = 1;
    expect(() => decodeRunPresentationV1(JSON.stringify(countDrift))).toThrow(
      RunPresentationDecodeError
    );

    const identityDrift = structuredClone(valid) as any;
    identityDrift.steps[0].forEach.iterations[1].itemNumber = 9;
    expect(() => decodeRunPresentationV1(JSON.stringify(identityDrift))).toThrow(
      RunPresentationDecodeError
    );

    const unbounded = structuredClone(valid) as any;
    unbounded.steps[0].forEach.total = 101;
    unbounded.steps[0].forEach.succeeded = 101;
    unbounded.steps[0].forEach.failed = 0;
    unbounded.steps[0].forEach.skipped = 0;
    unbounded.steps[0].forEach.iterations = Array.from(
      { length: 101 },
      (_, index) => ({ index, itemNumber: index + 1, status: 'succeeded' })
    );
    expect(() => decodeRunPresentationV1(JSON.stringify(unbounded))).toThrow(
      RunPresentationDecodeError
    );
  });

  test('redacts credentials from loop and per-item failure diagnostics', async () => {
    const output = renderRunPresentation(await loopPresentation(), { format: 'json' });
    expect(output).not.toContain('secret-token');
    expect(output).not.toContain('hidden-key');
    expect(output).not.toContain('xoxb-sensitive');
    expect(output).toContain('[redacted]');
    const parsed = JSON.parse(output);
    expect(parsed.steps[0].forEach.iterations[1]).not.toHaveProperty('item');
  });
});
