import type { CompiledWorkflowDefinition, ValueExpression } from 'woml';

import type {
  DiscordTriggerEventType,
  DiscordTriggerRegistration,
} from './types';

function fields(
  value: ValueExpression
): Readonly<Record<string, ValueExpression>> | undefined {
  return value.kind === 'object' ? value.fields : undefined;
}

function literalArray(value: ValueExpression | undefined): readonly string[] {
  if (value?.kind !== 'array') return [];
  return value.items.flatMap(item =>
    item.kind === 'literal' && typeof item.value === 'string'
      ? [item.value]
      : []
  );
}

export function discordTriggerRegistrations(
  workflow: CompiledWorkflowDefinition,
  definitionHash: string
): readonly DiscordTriggerRegistration[] {
  return workflow.triggers
    .filter(trigger => trigger.handler === 'trigger.discord')
    .map(trigger => {
      const config = fields(trigger.config);
      const events = literalArray(config?.events);
      const channels = literalArray(config?.channels);
      const botToken = config?.botToken?.kind === 'secretReference'
        ? config.botToken.name
        : undefined;
      if (
        config === undefined ||
        events.length === 0 ||
        events.some(
          event => event !== 'app-mention' && event !== 'direct-message'
        ) ||
        channels.some(channel => !/^[0-9]{17,20}$/.test(channel)) ||
        botToken === undefined
      ) {
        throw new Error(
          `Compiled Discord trigger "${trigger.id}" does not match Model v15.`
        );
      }
      return {
        workflowId: workflow.workflowId,
        definitionHash,
        triggerId: trigger.id,
        events: events as readonly DiscordTriggerEventType[],
        channels,
        credentialNames: { botToken },
      };
    });
}
