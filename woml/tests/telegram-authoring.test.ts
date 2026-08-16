import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import Ajv2020 from 'ajv/dist/2020';

import {
  buildWomlExecutableDefinitionPackage,
  buildWomlReusableDefinitionPackage,
  buildWomlRuntimeDefinitionPackage,
  compileWoml,
  generateWomlEditorDeclarations,
  inspectWomlMigrationDiagnostics,
  parseWoml,
  resolveWomlReusableDefinitionGraph,
  WomlDiagnosticError,
} from '../src';

const repositoryRoot = resolve(import.meta.dir, '../..');
const schemaRoot = resolve(repositoryRoot, 'docs/schemas');
const fixtureRoot = resolve(
  import.meta.dir,
  'fixtures/communication-providers'
);

function json(path: string): any {
  return JSON.parse(readFileSync(path, 'utf8'));
}

function document(name: string) {
  const path = resolve(fixtureRoot, name);
  return parseWoml(readFileSync(path, 'utf8'), { file: path });
}

function validators(): Ajv2020 {
  const ajv = new Ajv2020({ strict: false, allErrors: true });
  ajv.addFormat('date-time', /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z$/);
  ajv.addFormat('uri', /^https?:\/\/[^\s]+$/);
  const names = [
    ...Array.from({ length: 15 }, (_, index) =>
      `compiled-workflow-model.v${index + 1}.schema.json`
    ),
    ...Array.from({ length: 10 }, (_, index) =>
      `woml-definition-package.v${index + 1}.schema.json`
    ),
    'runtime-policy.v1.schema.json',
  ];
  for (const name of names) {
    ajv.addSchema(json(resolve(schemaRoot, name)));
  }
  return ajv;
}

function invalid(source: string): WomlDiagnosticError {
  try {
    compileWoml(parseWoml(source, { file: 'invalid-telegram.woml' }));
  } catch (error) {
    if (error instanceof WomlDiagnosticError) return error;
    throw error;
  }
  throw new Error('Expected invalid Telegram WOML.');
}

function workflow(trigger: string, body = 'return { ok: true };'): string {
  return `<woml>
  <workflow id="telegram-test" version="1.0.0">
    <triggers>${trigger}</triggers>
    <steps><step id="finish"><script>${body}</script></step></steps>
  </workflow>
</woml>`;
}

function approvalWorkflow(provider: string): string {
  return `<woml>
  <workflow id="telegram-approval" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps>
      <approval id="review">
        <notify>${provider}</notify>
        <when-approved />
        <when-rejected />
      </approval>
      <step id="finish"><script>return context.steps.review;</script></step>
    </steps>
  </workflow>
</woml>`;
}

describe('ACP2 Telegram authoring and lowering', () => {
  test('lowers the reviewed complete fixture into Model v15', () => {
    const compiled = compileWoml(document('telegram-acp2.woml'));
    expect(compiled.schemaVersion).toBe(15);
    if (compiled.schemaVersion !== 15) throw new Error('expected Model v15');

    const trigger = compiled.triggers[0];
    const approval = compiled.graph.nodes.find(node => node.id === 'review')!;
    if (approval.inputs.kind !== 'object') throw new Error('expected inputs');
    const notifications = approval.inputs.fields.notifications;
    if (notifications?.kind !== 'array') throw new Error('expected notifications');
    const approvalDestinations = notifications.items.map(item => {
      if (item.kind !== 'object') throw new Error('expected delivery');
      const destination = item.fields.destination;
      if (destination?.kind !== 'literal') throw new Error('expected destination');
      return destination.value;
    });
    if (trigger.config.kind !== 'object') throw new Error('expected config');
    const eventExpression = trigger.config.fields.events;
    const tokenExpression = trigger.config.fields.botToken;
    const expected = json(resolve(fixtureRoot, 'telegram-acp2.expected.json'));
    expect({
      schemaVersion: compiled.schemaVersion,
      trigger: {
        id: trigger.id,
        handler: trigger.handler,
        events:
          eventExpression?.kind === 'array'
            ? eventExpression.items.map(item =>
                item.kind === 'literal' ? item.value : null
              )
            : [],
        credential:
          tokenExpression?.kind === 'secretReference'
            ? tokenExpression.name
            : null,
      },
      approvalDestinations,
      notificationDeliveryIds:
        compiled.communication.providers[0].notificationDeliveryIds,
      communication: compiled.communication,
    }).toEqual(expected);

    const validate = validators().getSchema(
      'https://woml.dev/schemas/compiled-workflow-model/v15'
    )!;
    expect(validate(compiled), JSON.stringify(validate.errors, null, 2)).toBe(true);
  });

  test('discovers services.telegram in imported TypeScript and emits Package v10', async () => {
    const source = document('telegram-module-acp2.woml');
    const definitionPackage = await buildWomlExecutableDefinitionPackage(
      source,
      { sourcePath: source.file, projectRoot: fixtureRoot }
    );
    expect(definitionPackage).toMatchObject({
      schemaVersion: 10,
      profile: 'woml.definition-package/v10',
      executable: true,
      runtimeReady: false,
      workflow: { model: { schemaVersion: 15 } },
    });
    if (definitionPackage.schemaVersion !== 10) {
      throw new Error('expected Definition Package v10');
    }
    expect(definitionPackage.workflow.model.communication.providers[0]).toEqual({
      provider: 'telegram',
      triggerIds: [],
      notificationDeliveryIds: [],
      messaging: true,
      credentialNames: ['TELEGRAM_BOT_TOKEN'],
    });

    const validate = validators().getSchema(
      'https://woml.dev/schemas/woml-definition-package.v10.schema.json'
    )!;
    expect(
      validate(definitionPackage),
      JSON.stringify(validate.errors, null, 2)
    ).toBe(true);
    await expect(
      buildWomlRuntimeDefinitionPackage(source, {
        sourcePath: source.file,
        projectRoot: fixtureRoot,
      })
    ).resolves.toMatchObject({ runtimeReady: true });
  });

  test('requires imported Telegram messaging to remain rooted in an explicit workflow secret', async () => {
    const path = resolve(fixtureRoot, 'telegram-module-acp2.woml');
    const source = readFileSync(path, 'utf8').replace(
      'secrets.TELEGRAM_BOT_TOKEN',
      'context.payload.botToken'
    );
    const parsed = parseWoml(source, { file: path });
    try {
      await buildWomlExecutableDefinitionPackage(parsed, {
        sourcePath: path,
        projectRoot: fixtureRoot,
      });
      throw new Error('Expected unresolved Telegram credential rejection.');
    } catch (error) {
      expect(error).toBeInstanceOf(WomlDiagnosticError);
      expect((error as WomlDiagnosticError).diagnostic.code).toBe(
        'WOML_TELEGRAM_CREDENTIAL_UNRESOLVED'
      );
    }
  });

  test('reserves Discord and WhatsApp requirements in the same frozen Model v15 shape', () => {
    const base = compileWoml(document('telegram-acp2.woml')) as any;
    const validate = validators().getSchema(
      'https://woml.dev/schemas/compiled-workflow-model/v15'
    )!;
    const secret = (name: string) => ({ kind: 'secretReference', name });
    const literal = (value: string) => ({ kind: 'literal', value });
    const array = (values: string[]) => ({
      kind: 'array',
      items: values.map(literal),
    });
    for (const [provider, trigger] of [
      ['discord', {
        id: 'agentMessage',
        handler: 'trigger.discord',
        config: {
          kind: 'object',
          fields: {
            events: array(['app-mention', 'direct-message']),
            channels: array(['200000000000000002']),
            botToken: secret('DISCORD_BOT_TOKEN'),
          },
        },
      }],
      ['whatsapp', {
        id: 'customerMessage',
        handler: 'trigger.whatsapp',
        config: {
          kind: 'object',
          fields: {
            events: array(['message']),
            phoneNumberId: literal('123456789012345'),
            verifyToken: secret('WHATSAPP_VERIFY_TOKEN'),
            appSecret: secret('WHATSAPP_APP_SECRET'),
          },
        },
      }],
    ] as const) {
      const model = structuredClone(base);
      model.triggers = [trigger];
      model.communication = {
        profileVersion: 1,
        providers: [{
          provider,
          triggerIds: [trigger.id],
          notificationDeliveryIds: [],
          messaging: true,
          credentialNames:
            provider === 'discord'
              ? ['DISCORD_BOT_TOKEN']
              : ['WHATSAPP_APP_SECRET', 'WHATSAPP_VERIFY_TOKEN'],
        }],
      };
      expect(validate(model), JSON.stringify(validate.errors, null, 2)).toBe(true);
    }
  });

  test('preserves an explicit local services.telegram alias with a warning', async () => {
    const source = document('telegram-shadow-acp2.woml');
    const definitionPackage = await buildWomlExecutableDefinitionPackage(
      source,
      { sourcePath: source.file, projectRoot: fixtureRoot }
    );
    expect(definitionPackage.schemaVersion).toBe(2);
    expect(definitionPackage.workflow.model.schemaVersion).toBe(9);
    expect(inspectWomlMigrationDiagnostics(source)).toEqual([
      expect.objectContaining({
        severity: 'warning',
        code: 'WOML_BUILTIN_SERVICE_SHADOWED',
      }),
    ]);
  });

  test('resolves built-in Telegram triggers beside a local <telegram> notification provider', async () => {
    const source = document('telegram-contextual-alias-acp2.woml');
    const graph = resolveWomlReusableDefinitionGraph(source, {
      sourcePath: source.file,
      projectRoot: fixtureRoot,
    });
    const definitionPackage = await buildWomlReusableDefinitionPackage(
      source,
      graph,
      { sourcePath: source.file, projectRoot: fixtureRoot }
    );
    expect(definitionPackage.schemaVersion).toBe(10);
    expect(definitionPackage.runtimeReady).toBe(true);
    expect(definitionPackage.workflow.model.schemaVersion).toBe(15);
    expect(definitionPackage.workflow.model.triggers[0].handler).toBe(
      'trigger.telegram'
    );
    expect(definitionPackage.definitions).toContainEqual(
      expect.objectContaining({
        alias: 'telegram',
        kind: 'notification-provider',
      })
    );
    const serialized = JSON.stringify(definitionPackage.workflow.model);
    expect(serialized).toContain('"provider":{"kind":"literal","value":"custom"}');
    const validate = validators().getSchema(
      'https://woml.dev/schemas/woml-definition-package.v10.schema.json'
    )!;
    expect(
      validate(definitionPackage),
      JSON.stringify(validate.errors, null, 2)
    ).toBe(true);
  });

  test('reports source-aware Telegram grammar and service errors', () => {
    const cases = [
      [
        workflow('<telegram id="message" events="edited-message" bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}" />'),
        'WOML_TELEGRAM_TRIGGER_EVENT_INVALID',
      ],
      [
        workflow('<telegram id="message" events="message" bot-token="plaintext" />'),
        'WOML_SECRET_LITERAL_FORBIDDEN',
      ],
      [
        workflow('<telegram id="message" events="message" bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}" />',
          `return services.telegram.send({ botToken: secrets.TELEGRAM_BOT_TOKEN, conversationId: '1' });`),
        'WOML_TELEGRAM_SEND_PROPERTY_REQUIRED',
      ],
      [
        workflow('<manual id="start" />',
          `return services.telegram.edit({ botToken: secrets.TELEGRAM_BOT_TOKEN });`),
        'WOML_TELEGRAM_OPERATION_UNSUPPORTED',
      ],
      [
        approvalWorkflow('<telegram chats="team" bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}" />'),
        'WOML_TELEGRAM_CHAT_INVALID',
      ],
      [
        approvalWorkflow('<telegram chats="123,123" bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}" />'),
        'WOML_TELEGRAM_CHAT_DUPLICATE',
      ],
      [
        approvalWorkflow('<telegram bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}" />'),
        'WOML_TELEGRAM_ATTRIBUTE_REQUIRED',
      ],
      [
        approvalWorkflow('<telegram chats="123" message="Approve this" bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}" />'),
        'WOML_TELEGRAM_UNKNOWN_ATTRIBUTE',
      ],
      [
        `<woml><workflow id="telegram-placement"><triggers><manual id="start" /></triggers><steps><telegram chats="123" bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}" /></steps></workflow></woml>`,
        'WOML_NOTIFY_INVALID_ORDER',
      ],
    ] as const;
    for (const [source, code] of cases) {
      const error = invalid(source);
      expect(error.diagnostic.code).toBe(code);
      expect(error.diagnostic.location.start.line).toBeGreaterThan(0);
      expect(error.diagnostic.location.start.column).toBeGreaterThan(0);
    }
  });

  test('generates Telegram editor declarations', () => {
    const declarations = generateWomlEditorDeclarations([]);
    expect(declarations).toContain('interface WomlTelegramSendRequest');
    expect(declarations).toContain('readonly telegram:');
    expect(declarations).toContain('replyToMessageId?: string');
  });
});
