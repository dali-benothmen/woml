import type {
  CompiledWorkflowDefinition,
  ValueExpression,
} from 'woml';

import type {
  SlackTriggerEventType,
  SlackTriggerRegistration,
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

function secretName(value: ValueExpression | undefined): string | undefined {
  return value?.kind === 'secretReference' ? value.name : undefined;
}

export function slackTriggerRegistrations(
  workflow: CompiledWorkflowDefinition,
  definitionHash: string
): readonly SlackTriggerRegistration[] {
  return workflow.triggers
    .filter(trigger => trigger.handler === 'trigger.slack')
    .map(trigger => {
      const config = fields(trigger.config);
      const events = literalArray(config?.events);
      const channels = literalArray(config?.channels);
      const botToken = secretName(config?.botToken);
      const appToken = secretName(config?.appToken);
      if (
        config === undefined ||
        events.length === 0 ||
        events.some(
          event => event !== 'app-mention' && event !== 'direct-message'
        ) ||
        botToken === undefined ||
        appToken === undefined
      ) {
        throw new Error(
          `Compiled Slack trigger "${trigger.id}" does not match Model v7.`
        );
      }
      return {
        workflowId: workflow.workflowId,
        definitionHash,
        triggerId: trigger.id,
        events: events as readonly SlackTriggerEventType[],
        channels,
        credentialNames: { botToken, appToken },
      };
    });
}
