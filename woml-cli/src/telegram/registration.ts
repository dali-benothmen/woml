import type { CompiledWorkflowDefinition, ValueExpression } from '@woml/compiler';

import type { TelegramTriggerRegistration } from './types';

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

export function telegramTriggerRegistrations(
  workflow: CompiledWorkflowDefinition,
  definitionHash: string
): readonly TelegramTriggerRegistration[] {
  return workflow.triggers
    .filter(trigger => trigger.handler === 'trigger.telegram')
    .map(trigger => {
      const config = fields(trigger.config);
      const events = literalArray(config?.events);
      const botToken = config?.botToken?.kind === 'secretReference'
        ? config.botToken.name
        : undefined;
      if (
        config === undefined ||
        events.length !== 1 ||
        events[0] !== 'message' ||
        botToken === undefined
      ) {
        throw new Error(
          `Compiled Telegram trigger "${trigger.id}" does not match Model v15.`
        );
      }
      return {
        workflowId: workflow.workflowId,
        definitionHash,
        triggerId: trigger.id,
        events: ['message'],
        credentialNames: { botToken },
      };
    });
}
