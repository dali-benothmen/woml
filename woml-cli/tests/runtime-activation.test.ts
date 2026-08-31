import { describe, expect, test } from 'bun:test';

import {
  compileWoml,
  parseWoml,
  type CompiledWorkflowDefinition,
} from '@woml/compiler';

import { runtimeActivationRequirements } from '../src/runtime-activation';

function compile(source: string, name: string): CompiledWorkflowDefinition {
  return compileWoml(parseWoml(source, { file: `${name}.woml` }));
}

const simple = compile(
  `
<woml>
  <workflow id="simple" name="Simple" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps>
      <step id="hello"><script>return { message: 'hello' };</script></step>
    </steps>
  </workflow>
</woml>`,
  'simple'
);

function telegramLifecycle(): CompiledWorkflowDefinition {
  return compile(
    `
<woml>
  <workflow id="telegram-lifecycle" name="Telegram lifecycle" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps>
      <step id="hello"><script>return { message: 'hello' };</script></step>
    </steps>
    <lifecycle>
      <on-complete>
        <notify>
          <telegram chats="123456" message="Done" bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}" />
        </notify>
      </on-complete>
    </lifecycle>
  </workflow>
</woml>`,
    'telegram-lifecycle'
  );
}

function telegramApproval(): CompiledWorkflowDefinition {
  return compile(
    `
<woml>
  <workflow id="telegram-approval" name="Telegram approval" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps>
      <approval id="review" timeout="1h" on-timeout="reject">
        <notify>
          <telegram chats="123456" bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}" />
        </notify>
        <when-approved>
          <step id="approved"><script>return { approved: true };</script></step>
        </when-approved>
        <when-rejected>
          <step id="rejected"><script>return { approved: false };</script></step>
        </when-rejected>
      </approval>
    </steps>
  </workflow>
</woml>`,
    'telegram-approval'
  );
}

function forEachTelegramLifecycle(): CompiledWorkflowDefinition {
  return compile(
    `
<woml>
  <workflow id="for-each-notification" name="For each notification" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps>
      <step id="seed"><script>return { items: [1] };</script></step>
      <for-each id="items" items="{{context.steps.seed.items}}">
        <step id="visit"><script>return { item: context.item };</script></step>
      </for-each>
    </steps>
    <lifecycle>
      <on-complete>
        <notify>
          <telegram chats="123456" message="Done" bot-token="{{secrets.TELEGRAM_BOT_TOKEN}}" />
        </notify>
      </on-complete>
    </lifecycle>
  </workflow>
</woml>`,
    'for-each-notification'
  );
}

function discordTriggerAndNotification(): CompiledWorkflowDefinition {
  return compile(
    `
<woml>
  <workflow id="discord-mixed" name="Discord mixed" version="1.0.0">
    <triggers>
      <discord id="message" events="app-mention" bot-token="{{secrets.DISCORD_TRIGGER_TOKEN}}" />
    </triggers>
    <steps>
      <step id="reply"><script>return { message: context.payload.text };</script></step>
    </steps>
    <lifecycle>
      <on-complete>
        <notify>
          <discord channels="200000000000000001" message="Done" bot-token="{{secrets.DISCORD_NOTIFY_TOKEN}}" />
        </notify>
      </on-complete>
    </lifecycle>
  </workflow>
</woml>`,
    'discord-mixed'
  );
}

describe('compiled-model runtime activation requirements', () => {
  test('keeps a simple manual workflow free of optional ingress and providers', () => {
    expect(runtimeActivationRequirements([simple])).toEqual({
      profile: 'woml.runtime-activation-requirements/v1',
      triggerHandlers: ['trigger.manual'],
      publicHttp: false,
      scriptExecution: true,
      runtimeModules: false,
      providers: {
        telegram: {
          provider: 'telegram',
          used: false,
          inbound: false,
          triggerIds: [],
          approvalDeliveryIds: [],
          notificationDeliveryIds: [],
          messaging: false,
          credentialNames: [],
          inboundCredentialNames: [],
        },
        discord: {
          provider: 'discord',
          used: false,
          inbound: false,
          triggerIds: [],
          approvalDeliveryIds: [],
          notificationDeliveryIds: [],
          messaging: false,
          credentialNames: [],
          inboundCredentialNames: [],
        },
        whatsapp: {
          provider: 'whatsapp',
          used: false,
          inbound: false,
          triggerIds: [],
          approvalDeliveryIds: [],
          notificationDeliveryIds: [],
          messaging: false,
          credentialNames: [],
          inboundCredentialNames: [],
        },
      },
    });
  });

  test('does not start Telegram polling for an outbound-only lifecycle notification', () => {
    const requirement = runtimeActivationRequirements([telegramLifecycle()]);
    expect(requirement.providers.telegram).toMatchObject({
      used: true,
      inbound: false,
      triggerIds: [],
      approvalDeliveryIds: [],
      credentialNames: ['TELEGRAM_BOT_TOKEN'],
      inboundCredentialNames: [],
    });
    expect(requirement.providers.telegram.notificationDeliveryIds).toHaveLength(
      1
    );
  });

  test('starts Telegram inbound handling when approval buttons can return a decision', () => {
    const requirement = runtimeActivationRequirements([telegramApproval()]);
    expect(requirement.providers.telegram).toMatchObject({
      used: true,
      inbound: true,
      triggerIds: [],
      credentialNames: ['TELEGRAM_BOT_TOKEN'],
      inboundCredentialNames: ['TELEGRAM_BOT_TOKEN'],
    });
    expect(requirement.providers.telegram.approvalDeliveryIds).toHaveLength(1);
  });

  test('preserves provider requirements on Model v16 for-each workflows', () => {
    const workflow = forEachTelegramLifecycle();
    expect(workflow.schemaVersion).toBe(16);
    expect(
      runtimeActivationRequirements([workflow]).providers.telegram
    ).toMatchObject({
      used: true,
      inbound: false,
      credentialNames: ['TELEGRAM_BOT_TOKEN'],
      inboundCredentialNames: [],
    });
  });

  test('opens only credentials that belong to an inbound provider surface', () => {
    const requirement = runtimeActivationRequirements([
      discordTriggerAndNotification(),
    ]).providers.discord;
    expect(requirement).toMatchObject({
      used: true,
      inbound: true,
      triggerIds: ['message'],
      credentialNames: ['DISCORD_NOTIFY_TOKEN', 'DISCORD_TRIGGER_TOKEN'],
      inboundCredentialNames: ['DISCORD_TRIGGER_TOKEN'],
    });
  });

  test('merges multiple workflow requirements deterministically', () => {
    const first = runtimeActivationRequirements([simple, telegramApproval()]);
    const reversed = runtimeActivationRequirements([
      telegramApproval(),
      simple,
    ]);
    expect(reversed).toEqual(first);
  });
});
