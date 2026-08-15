import { describe, expect, test } from 'bun:test';
import { resolve } from 'node:path';

import {
  decodeRunPresentationV1,
  renderPresentationWarning,
  renderRunPresentation,
  RunPresentationDecodeError,
  sanitizeTerminalText,
  stripAnsi,
  type RunPresentationV1,
} from '../src/terminal-presentation';

const fixturePath = resolve(
  import.meta.dir,
  'fixtures/terminal-presentation/success.v1.json'
);

async function fixture(): Promise<RunPresentationV1> {
  return await Bun.file(fixturePath).json();
}

function terminalWidth(value: string): number {
  return [...stripAnsi(value)].reduce((width, character) => {
    const code = character.codePointAt(0)!;
    if (/\p{Mark}/u.test(character) || code === 0x200d || code === 0xfe0f) {
      return width;
    }
    const wide = code >= 0x1100 && (
      code <= 0x115f ||
      (code >= 0x2e80 && code <= 0xa4cf && code !== 0x303f) ||
      (code >= 0xac00 && code <= 0xd7a3) ||
      (code >= 0x1f300 && code <= 0x1faff)
    );
    return width + (wide ? 2 : 1);
  }, 0);
}

describe('terminal presentation hardening', () => {
  test('neutralizes terminal escapes, C1 controls, bidi overrides, and line separators', () => {
    const unsafe = [
      'safe', '\u001b]0;stolen title\u0007', '\u009b2J',
      '\u202eevil', '\u2066hidden', '\u2028next',
    ].join('');
    const sanitized = sanitizeTerminalText(unsafe);
    expect(sanitized).not.toContain('\u001b');
    expect(sanitized).not.toContain('\u009b');
    expect(sanitized).not.toContain('\u202e');
    expect(sanitized).not.toContain('\u2066');
    expect(sanitized).toContain('\nnext');
  });

  test('redacts secret-shaped keys and secret fragments from results and errors', async () => {
    const base = await fixture();
    const unsafe: RunPresentationV1 = {
      ...structuredClone(base),
      result: {
        botToken: 'xoxb-key-value',
        clientSecret: 'client-value',
        credential: 'credential-value',
        idempotencyKey: 'idem-value',
        approvalUrl: 'https://example.test/approve?token=url-value',
        note: 'Bearer bearer-value token=query-value xoxb-inline-value',
      },
      failure: {
        code: 'WOML_TEST_FAILED',
        message: 'Authorization failed with Bearer error-value and secret=message-value',
      },
    };
    const output = renderRunPresentation(unsafe, { format: 'json' });
    for (const forbidden of [
      'key-value', 'client-value', 'credential-value', 'idem-value',
      'url-value', 'bearer-value', 'query-value', 'inline-value',
      'error-value', 'message-value',
    ]) {
      expect(output).not.toContain(forbidden);
    }
    expect(output).toContain('[redacted]');

    const warning = renderPresentationWarning(
      'WOML_RUNTIME_FAILED',
      'request failed with xoxb-warning-value and token=warning-query',
      { format: 'plain' }
    );
    expect(warning).not.toContain('warning-value');
    expect(warning).not.toContain('warning-query');
  });

  test('bounds wide Unicode correctly at the minimum terminal width', async () => {
    const base = await fixture();
    const unicode: RunPresentationV1 = {
      ...structuredClone(base),
      workflow: {
        ...structuredClone(base.workflow),
        name: '注文処理 🚀'.repeat(20),
        description: `Cafe\u0301 ${'界'.repeat(80)}`,
      },
      result: { message: '✅'.repeat(200) },
    };
    const output = renderRunPresentation(unicode, {
      format: 'plain', width: 32, unicode: true, timeZone: 'UTC',
    });
    for (const line of output.trimEnd().split('\n')) {
      expect(terminalWidth(line), line).toBeLessThanOrEqual(32);
    }
  });

  test('fails closed on malformed, deeply nested, and oversized transport JSON', async () => {
    const base = await fixture();
    expect(() => decodeRunPresentationV1('{')).toThrow(RunPresentationDecodeError);
    expect(() => decodeRunPresentationV1('"' + '界'.repeat(800_000) + '"'))
      .toThrow(RunPresentationDecodeError);

    const deep = structuredClone(base) as unknown as Record<string, unknown>;
    deep.result = { a: { b: { c: { d: { e: { f: { g: true } } } } } } };
    expect(() => decodeRunPresentationV1(JSON.stringify(deep)))
      .toThrow(RunPresentationDecodeError);
  });

  test('keeps plain, JSON, and non-TTY output ANSI-free across color modes', async () => {
    const base = await fixture();
    const cases = [
      { format: 'plain' as const, color: 'always' as const, isTTY: true },
      { format: 'json' as const, color: 'always' as const, isTTY: true },
      { format: 'tty' as const, color: 'auto' as const, isTTY: false },
      {
        format: 'tty' as const,
        color: 'auto' as const,
        isTTY: true,
        environment: { TERM: 'dumb' },
      },
      {
        format: 'tty' as const,
        color: 'auto' as const,
        isTTY: true,
        environment: { TERM: 'xterm-256color', NO_COLOR: '1' },
      },
    ];
    for (const options of cases) {
      expect(renderRunPresentation(base, options)).not.toContain('\u001b[');
    }
    expect(renderRunPresentation(base, {
      format: 'tty', color: 'always', isTTY: false,
    })).toContain('\u001b[');
  });
});
