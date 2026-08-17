import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import {
  assertCustomProviderCompleted,
  assertCustomProviderInbound,
  assertCustomProviderOutbound,
  CustomProviderProtocolError,
  type CustomProviderCompletedMessage,
  type CustomProviderFailureKind,
} from '../src/custom-notification-provider';
import {
  encodeFrame,
  FrameDecoder,
  FrameProtocolError,
} from '../src/script-host/framing';

const fixture = JSON.parse(
  readFileSync(
    resolve(
      import.meta.dir,
      '../../woml/tests/fixtures/reusable-definitions/provider-protocol.v1.json'
    ),
    'utf8'
  )
) as Record<string, any>;

const failureKinds: readonly CustomProviderFailureKind[] = [
  'script_threw',
  'timed_out',
  'cancelled',
  'non_json',
  'worker_crashed',
  'host_crashed',
  'context_too_large',
  'result_too_large',
  'delivery_ambiguous',
  'service_failed',
  'request_invalid',
];

describe('Custom Notification Provider Protocol v1', () => {
  test('accepts execute/cancel inbound and ready/completed outbound directions', () => {
    assertCustomProviderInbound(
      fixture.execute,
      new Set([fixture.execute.scriptArtifactId])
    );
    assertCustomProviderInbound(fixture.cancel);
    assertCustomProviderOutbound(fixture.ready);
    assertCustomProviderCompleted(fixture.completed);

    expect(() => assertCustomProviderInbound(fixture.ready)).toThrow(
      CustomProviderProtocolError
    );
    expect(() => assertCustomProviderOutbound(fixture.execute)).toThrow(
      CustomProviderProtocolError
    );
  });

  test('frames multibyte UTF-8 and literal CRLF by bytes', () => {
    const first = encodeFrame(fixture.execute);
    const second = encodeFrame(fixture.cancel);
    const headerEnd = first.indexOf(Buffer.from('\r\n\r\n'));
    const declared = Number(
      first.subarray(0, headerEnd).toString('ascii').split(': ')[1]
    );
    expect(declared).toBe(first.byteLength - headerEnd - 4);
    expect(declared).toBeGreaterThan(JSON.stringify(fixture.execute).length);

    const decoder = new FrameDecoder();
    const joined = Buffer.concat([first, second]);
    const split = Math.floor(joined.byteLength / 2);
    expect(decoder.push(joined.subarray(0, split))).toHaveLength(0);
    expect(decoder.push(joined.subarray(split))).toEqual([
      fixture.execute,
      fixture.cancel,
    ]);
    decoder.finish();
  });

  test('correlates multiplexed completions by invocationId out of order', () => {
    const first = {
      ...fixture.completed,
      invocationId: 'provider_invocation_1',
    };
    const second = {
      ...fixture.completed,
      invocationId: 'provider_invocation_2',
    };
    for (const completion of [second, first]) {
      assertCustomProviderCompleted(completion);
    }
    expect([second, first].map(item => item.invocationId)).toEqual([
      'provider_invocation_2',
      'provider_invocation_1',
    ]);
  });

  test('rejects unknown artifacts, invalid attempts, and oversized frames', () => {
    expect(() =>
      assertCustomProviderInbound(fixture.execute, new Set(['another-artifact']))
    ).toThrow('is not registered');
    expect(() =>
      assertCustomProviderInbound({
        ...fixture.execute,
        attempt: { number: 4, max: 3 },
      })
    ).toThrow('must not exceed');

    const decoder = new FrameDecoder({ maxFrameBytes: 16 });
    expect(() => decoder.push(encodeFrame(fixture.execute))).toThrow(
      FrameProtocolError
    );
  });

  test('rejects malformed or response-bearing receipts', () => {
    for (const receipt of [
      { messageId: '' },
      { messageId: 'ok', responseBody: 'secret response' },
      { providerResponse: { token: 'secret' } },
    ]) {
      expect(() =>
        assertCustomProviderCompleted({
          ...fixture.completed,
          outcome: { kind: 'succeeded', receipt },
        })
      ).toThrow(CustomProviderProtocolError);
    }
  });

  test('accepts every frozen structured failure kind without sensitive fields', () => {
    for (const kind of failureKinds) {
      const completed: CustomProviderCompletedMessage = {
        ...fixture.failed,
        invocationId: `failure_${kind}`,
        outcome: {
          kind: 'failed',
          error: {
            kind,
            code: `WOML_PROVIDER_${kind.toUpperCase()}`,
            message: `Safe ${kind} failure.`,
            retryable: kind === 'service_failed',
          },
        },
      };
      assertCustomProviderCompleted(completed);
      expect(JSON.stringify(completed)).not.toMatch(
        /botToken|approvalUrl|rejectUrl|responseBody|secretValue|"props"|"notification"/
      );
    }
  });
});
