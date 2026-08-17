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
  for (const name of [
    ...Array.from({ length: 15 }, (_, index) =>
      `compiled-workflow-model.v${index + 1}.schema.json`
    ),
    ...Array.from({ length: 10 }, (_, index) =>
      `woml-definition-package.v${index + 1}.schema.json`
    ),
    'runtime-policy.v1.schema.json',
  ]) {
    ajv.addSchema(json(resolve(schemaRoot, name)));
  }
  return ajv;
}

function invalid(source: string): WomlDiagnosticError {
  try {
    compileWoml(parseWoml(source, { file: 'invalid-discord.woml' }));
  } catch (error) {
    if (error instanceof WomlDiagnosticError) return error;
    throw error;
  }
  throw new Error('Expected invalid Discord WOML.');
}

function workflow(trigger: string, body = 'return { ok: true };'): string {
  return `<woml>
  <workflow id="discord-test" version="1.0.0">
    <triggers>${trigger}</triggers>
    <steps><step id="finish"><script>${body}</script></step></steps>
  </workflow>
</woml>`;
}

function approvalWorkflow(provider: string): string {
  return `<woml>
  <workflow id="discord-approval" version="1.0.0">
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

describe('Discord authoring and lowering', () => {
  test('lowers the reviewed complete fixture into schema-valid Model v15', () => {
    const compiled = compileWoml(document('discord.woml'));
    expect(compiled.schemaVersion).toBe(15);
    if (compiled.schemaVersion !== 15) throw new Error('expected Model v15');

    const trigger = compiled.triggers[0];
    const approval = compiled.graph.nodes.find(node => node.id === 'review')!;
    if (trigger.config.kind !== 'object' || approval.inputs.kind !== 'object') {
      throw new Error('expected object expressions');
    }
    const notifications = approval.inputs.fields.notifications;
    if (notifications?.kind !== 'array') throw new Error('expected notifications');
    const literalArray = (value: unknown): unknown[] =>
      typeof value === 'object' && value !== null &&
      'kind' in value && value.kind === 'array' && 'items' in value &&
      Array.isArray(value.items)
        ? value.items.map(item =>
            item.kind === 'literal' ? item.value : null
          )
        : [];
    const literal = (value: unknown): unknown =>
      typeof value === 'object' && value !== null &&
      'kind' in value && value.kind === 'literal' && 'value' in value
        ? value.value
        : null;
    const secret = (value: unknown): unknown =>
      typeof value === 'object' && value !== null &&
      'kind' in value && value.kind === 'secretReference' && 'name' in value
        ? value.name
        : null;
    const actual = {
      schemaVersion: compiled.schemaVersion,
      trigger: {
        id: trigger.id,
        handler: trigger.handler,
        events: literalArray(trigger.config.fields.events),
        channels: literalArray(trigger.config.fields.channels),
        credential: secret(trigger.config.fields.botToken),
      },
      approvalDestinations: notifications.items.map(item =>
        item.kind === 'object' ? literal(item.fields.destination) : null
      ),
      notificationDeliveryIds:
        compiled.communication.providers[0].notificationDeliveryIds,
      communication: compiled.communication,
    };
    expect(actual).toEqual(
      json(resolve(fixtureRoot, 'discord.expected.json'))
    );

    const validate = validators().getSchema(
      'https://woml.dev/schemas/compiled-workflow-model/v15'
    )!;
    expect(validate(compiled), JSON.stringify(validate.errors, null, 2)).toBe(true);
  });

  test('discovers services.discord in imported TypeScript and promotes it for runtime', async () => {
    const source = document('discord-module.woml');
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
    expect(definitionPackage.workflow.model.communication.providers).toEqual([
      {
        provider: 'discord',
        triggerIds: [],
        notificationDeliveryIds: [],
        messaging: true,
        credentialNames: ['DISCORD_BOT_TOKEN'],
      },
    ]);

    const validate = validators().getSchema(
      'https://woml.dev/schemas/woml-definition-package.v10.schema.json'
    )!;
    expect(validate(definitionPackage), JSON.stringify(validate.errors, null, 2)).toBe(true);
    const runtimePackage = await buildWomlRuntimeDefinitionPackage(source, {
      sourcePath: source.file,
      projectRoot: fixtureRoot,
    });
    expect(runtimePackage.schemaVersion).toBe(10);
    expect(runtimePackage.runtimeReady).toBe(true);
  });

  test('requires imported Discord messaging credentials to be explicit workflow secrets', async () => {
    const path = resolve(fixtureRoot, 'discord-module.woml');
    const source = readFileSync(path, 'utf8').replace(
      'secrets.DISCORD_BOT_TOKEN',
      'context.payload.botToken'
    );
    try {
      await buildWomlExecutableDefinitionPackage(parseWoml(source, { file: path }), {
        sourcePath: path,
        projectRoot: fixtureRoot,
      });
      throw new Error('Expected unresolved Discord credential rejection.');
    } catch (error) {
      expect(error).toBeInstanceOf(WomlDiagnosticError);
      expect((error as WomlDiagnosticError).diagnostic.code).toBe(
        'WOML_DISCORD_CREDENTIAL_UNRESOLVED'
      );
    }
  });

  test('preserves an explicit local services.discord alias with a warning', async () => {
    const source = document('discord-shadow.woml');
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

  test('resolves a built-in Discord trigger beside a local <discord> provider', async () => {
    const source = document('discord-contextual-alias.woml');
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
      'trigger.discord'
    );
    expect(definitionPackage.definitions).toContainEqual(
      expect.objectContaining({
        alias: 'discord',
        kind: 'notification-provider',
      })
    );
    expect(JSON.stringify(definitionPackage.workflow.model)).toContain(
      '"provider":{"kind":"literal","value":"custom"}'
    );
  });

  test('reports source-aware Discord grammar and service errors', () => {
    const cases = [
      [
        workflow('<discord id="message" events="slash-command" bot-token="{{secrets.DISCORD_BOT_TOKEN}}" />'),
        'WOML_DISCORD_TRIGGER_EVENT_INVALID',
      ],
      [
        workflow('<discord id="message" events="app-mention" channels="woml-testing" bot-token="{{secrets.DISCORD_BOT_TOKEN}}" />'),
        'WOML_DISCORD_CHANNEL_INVALID',
      ],
      [
        workflow('<discord id="message" events="direct-message" bot-token="plaintext" />'),
        'WOML_SECRET_LITERAL_FORBIDDEN',
      ],
      [
        workflow('<manual id="start" />', `return services.discord.send({ botToken: secrets.DISCORD_BOT_TOKEN, conversationId: '200000000000000001' });`),
        'WOML_DISCORD_SEND_PROPERTY_REQUIRED',
      ],
      [
        workflow('<manual id="start" />', `return services.discord.send({ botToken: secrets.DISCORD_BOT_TOKEN, conversationId: 'general', text: 'hello' });`),
        'WOML_DISCORD_SEND_VALUE_INVALID',
      ],
      [
        workflow('<manual id="start" />', `return services.discord.edit({ botToken: secrets.DISCORD_BOT_TOKEN });`),
        'WOML_DISCORD_OPERATION_UNSUPPORTED',
      ],
      [
        workflow('<manual id="start" />', `return services.discord.send({ botToken: secrets.DISCORD_BOT_TOKEN, conversationId: '200000000000000001', text: 'hello' }, { name: 'Reply Now' });`),
        'WOML_DISCORD_SEND_NAME_INVALID',
      ],
      [
        approvalWorkflow('<discord channels="general" bot-token="{{secrets.DISCORD_BOT_TOKEN}}" />'),
        'WOML_DISCORD_CHANNEL_INVALID',
      ],
      [
        approvalWorkflow('<discord channels="200000000000000001,200000000000000001" bot-token="{{secrets.DISCORD_BOT_TOKEN}}" />'),
        'WOML_DISCORD_CHANNEL_DUPLICATE',
      ],
      [
        approvalWorkflow('<discord bot-token="{{secrets.DISCORD_BOT_TOKEN}}" />'),
        'WOML_DISCORD_ATTRIBUTE_REQUIRED',
      ],
    ] as const;
    for (const [source, code] of cases) {
      const error = invalid(source);
      expect(error.diagnostic.code).toBe(code);
      expect(error.diagnostic.location.start.line).toBeGreaterThan(0);
      expect(error.diagnostic.location.start.column).toBeGreaterThan(0);
    }
  });

  test('generates Discord editor declarations', () => {
    const declarations = generateWomlEditorDeclarations([]);
    expect(declarations).toContain('interface WomlDiscordSendRequest');
    expect(declarations).toContain('readonly discord:');
    expect(declarations).toContain('replyToMessageId?: string');

    const snippets = json(
      resolve(repositoryRoot, 'woml-vscode/snippets/woml.code-snippets')
    );
    expect(snippets).toHaveProperty('WOML Discord trigger');
    expect(snippets).toHaveProperty('WOML Discord message');
  });
});
