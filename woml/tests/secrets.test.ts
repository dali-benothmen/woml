import { describe, expect, test } from 'bun:test';

import {
  compileWoml,
  isValidSecretName,
  parseSecretReference,
  parseWoml,
  requireSecretReference,
  WomlValidationError,
} from '../src';

function workflow(item: string): string {
  return `<workflow id="secret-test">
  <triggers><manual id="start" /></triggers>
  <steps>${item}</steps>
</workflow>`;
}

describe('WOML secret references', () => {
  test('parses only an exact whole-attribute reference', () => {
    expect(parseSecretReference('{{secrets.SLACK_BOT_TOKEN}}')).toEqual({
      kind: 'secretReference',
      name: 'SLACK_BOT_TOKEN',
    });
    expect(parseSecretReference('prefix {{secrets.SLACK_BOT_TOKEN}}')).toBe(
      undefined
    );
    expect(parseSecretReference('{{ secrets.SLACK_BOT_TOKEN }}')).toBe(
      undefined
    );
    expect(parseSecretReference('{{secrets.slack_token}}')).toBe(undefined);
    expect(parseSecretReference('{{context.steps.token}}')).toBe(undefined);
  });

  test('freezes the secret-name grammar', () => {
    expect(isValidSecretName('A')).toBe(true);
    expect(isValidSecretName('SLACK_BOT_TOKEN_2')).toBe(true);
    expect(isValidSecretName('2_TOKEN')).toBe(false);
    expect(isValidSecretName('slack_token')).toBe(false);
    expect(isValidSecretName('SLACK-TOKEN')).toBe(false);
    expect(isValidSecretName('SLACK.TOKEN')).toBe(false);
  });

  test('reports malformed references at the original attribute value', () => {
    const source = `<slack bot-token="{{secrets.slack_token}}" />`;
    const document = parseWoml(source, { file: 'notification.woml' });
    const attribute = document.root.attributes['bot-token'];

    try {
      requireSecretReference(document, attribute);
      throw new Error('Expected invalid secret reference.');
    } catch (error) {
      expect(error).toBeInstanceOf(WomlValidationError);
      const diagnostic = (error as WomlValidationError).diagnostic;
      expect(diagnostic.code).toBe('WOML_SECRET_REFERENCE_INVALID');
      expect(diagnostic.location.start).toEqual({
        line: 1,
        column: source.indexOf('{{secrets.') + 1,
        offset: source.indexOf('{{secrets.'),
      });
      expect(diagnostic.message).not.toContain('slack_token');
    }
  });

  test('rejects literal credentials without echoing the value', () => {
    const plaintext = 'xoxb-do-not-print-this';
    const source = `<slack bot-token="${plaintext}" />`;
    const document = parseWoml(source, { file: 'notification.woml' });

    expect(() =>
      requireSecretReference(document, document.root.attributes['bot-token'])
    ).toThrow(WomlValidationError);
    try {
      requireSecretReference(document, document.root.attributes['bot-token']);
    } catch (error) {
      const serialized = JSON.stringify(
        (error as WomlValidationError).diagnostic
      );
      expect(serialized).not.toContain(plaintext);
      expect((error as WomlValidationError).diagnostic.code).toBe(
        'WOML_SECRET_LITERAL_FORBIDDEN'
      );
    }
  });

  test('allows references only in reviewed Slack credential sinks', () => {
    const validButStaged = workflow(`<approval id="review">
      <notify>
        <slack channels="#approvals" bot-token="{{secrets.SLACK_BOT_TOKEN}}" app-token="{{secrets.SLACK_APP_TOKEN}}" />
      </notify>
      <when-approved />
      <when-rejected />
    </approval>`);
    try {
      compileWoml(parseWoml(validButStaged, { file: 'workflow.woml' }));
      throw new Error('Expected notify to remain staged until N2.');
    } catch (error) {
      expect(error).toBeInstanceOf(WomlValidationError);
      expect((error as WomlValidationError).diagnostic.code).toBe(
        'WOML_FEATURE_NOT_EXECUTABLE'
      );
    }

    const unsupported = workflow(
      '<step id="a" name="{{secrets.SECRET_NAME}}"><script>return true;</script></step>'
    );
    try {
      compileWoml(parseWoml(unsupported, { file: 'workflow.woml' }));
      throw new Error('Expected unsupported secret sink.');
    } catch (error) {
      expect(error).toBeInstanceOf(WomlValidationError);
      expect((error as WomlValidationError).diagnostic.code).toBe(
        'WOML_SECRET_SINK_UNSUPPORTED'
      );
    }
  });

  test('validates Slack credential values before the staged-feature error', () => {
    for (const [value, code] of [
      ['{{secrets.lowercase}}', 'WOML_SECRET_REFERENCE_INVALID'],
      ['xoxb-forbidden-literal', 'WOML_SECRET_LITERAL_FORBIDDEN'],
    ] as const) {
      const source = workflow(`<approval id="review">
        <notify>
          <slack channels="#approvals" bot-token="${value}" app-token="{{secrets.SLACK_APP_TOKEN}}" />
        </notify>
        <when-approved />
        <when-rejected />
      </approval>`);
      try {
        compileWoml(parseWoml(source, { file: 'workflow.woml' }));
        throw new Error('Expected invalid Slack secret input.');
      } catch (error) {
        expect(error).toBeInstanceOf(WomlValidationError);
        expect((error as WomlValidationError).diagnostic.code).toBe(code);
        expect(
          JSON.stringify((error as WomlValidationError).diagnostic)
        ).not.toContain('xoxb-forbidden-literal');
      }
    }
  });
});
