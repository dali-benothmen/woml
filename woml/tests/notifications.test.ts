import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

import {
  compileWoml,
  inspectCompiledWorkflowGraph,
  parseWoml,
  WomlValidationError,
  type CompiledWorkflowDefinition,
  type WomlDiagnostic,
} from '../src';

function workflow(item: string): string {
  return `<workflow id="notification-test">
  <triggers><manual id="start" /></triggers>
  <steps>${item}</steps>
</workflow>`;
}

function approval(notify: string, id = 'review'): string {
  return `<approval id="${id}">
    ${notify}
    <when-approved />
    <when-rejected />
  </approval>`;
}

function slack(
  channels = '#approvals',
  botToken = 'SLACK_BOT_TOKEN',
  appToken = 'SLACK_APP_TOKEN'
): string {
  return `<slack channels="${channels}" bot-token="{{secrets.${botToken}}}" app-token="{{secrets.${appToken}}}" />`;
}

function validationDiagnostic(source: string): WomlDiagnostic {
  try {
    compileWoml(parseWoml(source, { file: 'notification.woml' }));
  } catch (error) {
    expect(error).toBeInstanceOf(WomlValidationError);
    return (error as WomlValidationError).diagnostic;
  }
  throw new Error('Expected WOML validation to fail.');
}

function notificationItems(compiled: CompiledWorkflowDefinition) {
  const wait = compiled.graph.nodes.find(node => node.id === 'review');
  expect(wait?.inputs.kind).toBe('object');
  if (wait?.inputs.kind !== 'object') throw new Error('Missing wait inputs.');
  const notifications = wait.inputs.fields.notifications;
  expect(notifications?.kind).toBe('array');
  if (notifications?.kind !== 'array') {
    throw new Error('Missing notification deliveries.');
  }
  return notifications.items;
}

describe('WOML Slack notification lowering', () => {
  test('deep-equals the reviewed Model v5 fixture', () => {
    const source = readFileSync(
      new URL('./fixtures/approval-slack.woml', import.meta.url),
      'utf8'
    );
    const expected = JSON.parse(
      readFileSync(
        new URL('./fixtures/approval-slack.compiled.v5.json', import.meta.url),
        'utf8'
      )
    );

    const compiled = compileWoml(
      parseWoml(source, { file: 'approval-slack.woml' })
    );

    expect(compiled).toEqual(expected);
    expect(
      inspectCompiledWorkflowGraph(compiled.graph, {
        requireSingleTerminal: true,
      })
    ).toEqual([]);
  });

  test('expands ordered channels and separate workspaces deterministically', () => {
    const source = workflow(
      approval(`<notify>
        ${slack('#approvals C0123456789')}
        ${slack('#approvals', 'SECOND_BOT_TOKEN', 'SECOND_APP_TOKEN')}
      </notify>`)
    );
    const compiled = compileWoml(
      parseWoml(source, { file: 'notification.woml' })
    );

    expect(compiled.schemaVersion).toBe(5);
    expect(notificationItems(compiled)).toEqual([
      {
        kind: 'object',
        fields: {
          deliveryId: {
            kind: 'literal',
            value: 'review:notify:0:channel:0',
          },
          provider: { kind: 'literal', value: 'slack' },
          destination: { kind: 'literal', value: '#approvals' },
          credentials: {
            kind: 'object',
            fields: {
              botToken: {
                kind: 'secretReference',
                name: 'SLACK_BOT_TOKEN',
              },
              appToken: {
                kind: 'secretReference',
                name: 'SLACK_APP_TOKEN',
              },
            },
          },
        },
      },
      expect.objectContaining({
        fields: expect.objectContaining({
          deliveryId: {
            kind: 'literal',
            value: 'review:notify:0:channel:1',
          },
          destination: { kind: 'literal', value: 'C0123456789' },
        }),
      }),
      expect.objectContaining({
        fields: expect.objectContaining({
          deliveryId: {
            kind: 'literal',
            value: 'review:notify:1:channel:0',
          },
          destination: { kind: 'literal', value: '#approvals' },
          credentials: {
            kind: 'object',
            fields: {
              botToken: {
                kind: 'secretReference',
                name: 'SECOND_BOT_TOKEN',
              },
              appToken: {
                kind: 'secretReference',
                name: 'SECOND_APP_TOKEN',
              },
            },
          },
        }),
      }),
    ]);
  });

  test('finds notifications nested in approval decision routes', () => {
    const nested = approval(`<notify>${slack()}</notify>`, 'innerReview');
    const source = workflow(`<approval id="outerReview">
      <when-approved>${nested}</when-approved>
      <when-rejected />
    </approval>`);

    const compiled = compileWoml(
      parseWoml(source, { file: 'notification.woml' })
    );
    expect(compiled.schemaVersion).toBe(5);
    expect(inspectCompiledWorkflowGraph(compiled.graph)).toEqual([]);
  });

  test('composes notification approvals at root, branch-arm, nested, and parallel-adjacent placements', () => {
    const notificationApproval = (id: string) =>
      approval(`<notify>${slack()}</notify>`, id);
    const cases = [
      workflow(notificationApproval('rootReview')),
      workflow(`<step id="chooseRoute"><script>return true;</script></step>
        <branch id="route">
          <when test="{{context.steps.chooseRoute}}">
            ${notificationApproval('branchReview')}
            <result value="{{context.steps.branchReview}}" />
          </when>
          <otherwise>
            <step id="fallback"><script>return { decision: 'rejected' };</script></step>
            <result value="{{context.steps.fallback}}" />
          </otherwise>
        </branch>`),
      workflow(`<approval id="outerReview">
          <when-approved>
            ${notificationApproval('nestedReview')}
          </when-approved>
          <when-rejected />
        </approval>`),
      workflow(`<parallel id="checks" concurrency="2" on-error="wait-all">
          <step id="left"><script>return 1;</script></step>
          <step id="right"><script>return 2;</script></step>
        </parallel>
        ${notificationApproval('afterParallelReview')}
        <step id="afterReview"><script>return context.steps.afterParallelReview;</script></step>`),
    ];

    for (const source of cases) {
      const compiled = compileWoml(
        parseWoml(source, { file: 'notification-composition.woml' })
      );
      expect(compiled.schemaVersion).toBe(5);
      expect(inspectCompiledWorkflowGraph(compiled.graph)).toEqual([]);
      expect(
        compiled.graph.nodes.some(
          node =>
            node.handler === 'engine.approval-wait' &&
            node.inputs.kind === 'object' &&
            node.inputs.fields.notifications?.kind === 'array'
        )
      ).toBe(true);
    }
  });

  test('rejects invalid placement, provider structure, and Slack attributes', () => {
    const cases = [
      [workflow(`<notify>${slack()}</notify>`), 'WOML_NOTIFY_INVALID_ORDER'],
      [workflow(approval('<notify />')), 'WOML_NOTIFY_EMPTY'],
      [
        workflow(approval('<notify><discord /></notify>')),
        'WOML_NOTIFY_UNSUPPORTED_PROVIDER',
      ],
      [
        workflow(`<approval id="review">
          <when-approved />
          <notify>${slack()}</notify>
          <when-rejected />
        </approval>`),
        'WOML_NOTIFY_INVALID_ORDER',
      ],
      [
        workflow(
          approval(
            `<notify>${slack()}</notify><notify>${slack('#ops')}</notify>`
          )
        ),
        'WOML_NOTIFY_INVALID_ORDER',
      ],
      [
        workflow(
          approval(
            '<notify><slack bot-token="{{secrets.SLACK_BOT_TOKEN}}" app-token="{{secrets.SLACK_APP_TOKEN}}" /></notify>'
          )
        ),
        'WOML_SLACK_ATTRIBUTE_REQUIRED',
      ],
      [
        workflow(
          approval(
            `<notify><slack channels="#approvals" token="unused" bot-token="{{secrets.SLACK_BOT_TOKEN}}" app-token="{{secrets.SLACK_APP_TOKEN}}" /></notify>`
          )
        ),
        'WOML_SLACK_UNKNOWN_ATTRIBUTE',
      ],
      [
        workflow(approval(`<notify>${slack('   ')}</notify>`)),
        'WOML_SLACK_CHANNELS_EMPTY',
      ],
      [
        workflow(approval(`<notify>${slack('#Uppercase')}</notify>`)),
        'WOML_SLACK_CHANNEL_INVALID',
      ],
      [
        workflow(
          approval(`<notify>${slack('#approvals #approvals')}</notify>`)
        ),
        'WOML_SLACK_CHANNEL_DUPLICATE',
      ],
      [
        workflow(
          approval(
            `<notify>${slack('#approvals')}${slack('#approvals')}</notify>`
          )
        ),
        'WOML_SLACK_CHANNEL_DUPLICATE',
      ],
    ] as const;

    for (const [source, code] of cases) {
      expect(validationDiagnostic(source).code).toBe(code);
    }
  });

  test('points an invalid channel and duplicate at the exact token', () => {
    const invalid = workflow(
      approval(`<notify>${slack('#approvals BAD')}</notify>`)
    );
    expect(validationDiagnostic(invalid).location.start.offset).toBe(
      invalid.indexOf('BAD')
    );

    const duplicate = workflow(
      approval(`<notify>${slack('#approvals #approvals')}</notify>`)
    );
    expect(validationDiagnostic(duplicate).location.start.offset).toBe(
      duplicate.lastIndexOf('#approvals')
    );
  });

  test('graph inspection rejects malformed notification contracts', () => {
    const source = workflow(approval(`<notify>${slack()}</notify>`));
    const compiled = structuredClone(
      compileWoml(parseWoml(source, { file: 'notification.woml' }))
    ) as unknown as {
      graph: CompiledWorkflowDefinition['graph'];
    };
    const wait = compiled.graph.nodes.find(
      node => node.id === 'review'
    ) as unknown as {
      inputs: {
        fields: {
          notifications: {
            items: Array<{ fields: { deliveryId: { value: string } } }>;
          };
        };
      };
    };
    wait.inputs.fields.notifications.items[0].fields.deliveryId.value =
      'review:notify:9:channel:4';

    expect(inspectCompiledWorkflowGraph(compiled.graph)).toContainEqual(
      expect.objectContaining({ code: 'INVALID_NOTIFICATION_GROUP' })
    );
  });
});
