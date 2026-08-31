import type {
  CompiledWorkflowDefinition,
  CompiledWorkflowNode,
  ValueExpression,
} from '@woml/compiler';

export const RUNTIME_ACTIVATION_REQUIREMENTS_PROFILE =
  'woml.runtime-activation-requirements/v1' as const;

export type BuiltInCommunicationProvider = 'telegram' | 'discord' | 'whatsapp';

export interface ProviderActivationRequirementV1 {
  readonly provider: BuiltInCommunicationProvider;
  readonly used: boolean;
  /** An inbound connection is required for triggers or approval decisions. */
  readonly inbound: boolean;
  readonly triggerIds: readonly string[];
  readonly approvalDeliveryIds: readonly string[];
  readonly notificationDeliveryIds: readonly string[];
  readonly messaging: boolean;
  readonly credentialNames: readonly string[];
  readonly inboundCredentialNames: readonly string[];
}

export interface RuntimeActivationRequirementsV1 {
  readonly profile: typeof RUNTIME_ACTIVATION_REQUIREMENTS_PROFILE;
  readonly triggerHandlers: readonly string[];
  readonly publicHttp: boolean;
  readonly scriptExecution: boolean;
  readonly runtimeModules: boolean;
  readonly providers: Readonly<
    Record<BuiltInCommunicationProvider, ProviderActivationRequirementV1>
  >;
}

interface CommunicationRequirement {
  readonly provider: BuiltInCommunicationProvider;
  readonly triggerIds: readonly string[];
  readonly notificationDeliveryIds: readonly string[];
  readonly messaging: boolean;
  readonly credentialNames: readonly string[];
}

function communicationRequirements(
  workflow: CompiledWorkflowDefinition
): readonly CommunicationRequirement[] {
  if (!('communication' in workflow) || workflow.communication === undefined)
    return [];
  return workflow.communication.providers;
}

function workflowNodes(
  workflow: CompiledWorkflowDefinition
): readonly CompiledWorkflowNode[] {
  return [
    ...workflow.graph.nodes,
    ...(workflow.schemaVersion === 16
      ? workflow.graph.forEach.flatMap(descriptor => descriptor.body.nodes)
      : []),
  ];
}

function literalString(
  expression: ValueExpression | undefined
): string | undefined {
  return expression?.kind === 'literal' && typeof expression.value === 'string'
    ? expression.value
    : undefined;
}

function collectSecretNames(
  expression: ValueExpression | undefined,
  names: Set<string>
): void {
  if (expression === undefined) return;
  if (expression.kind === 'secretReference') {
    names.add(expression.name);
    return;
  }
  if (expression.kind === 'array') {
    for (const item of expression.items) collectSecretNames(item, names);
    return;
  }
  if (expression.kind === 'object') {
    for (const value of Object.values(expression.fields))
      collectSecretNames(value, names);
  }
}

function inboundCredentialNames(
  provider: BuiltInCommunicationProvider,
  workflow: CompiledWorkflowDefinition
): readonly string[] {
  const names = new Set<string>();
  for (const trigger of workflow.triggers) {
    if (trigger.handler === `trigger.${provider}`)
      collectSecretNames(trigger.config, names);
  }
  for (const node of workflowNodes(workflow)) {
    if (
      node.handler !== 'engine.approval-wait' ||
      node.inputs.kind !== 'object'
    )
      continue;
    const notifications = node.inputs.fields.notifications;
    if (notifications?.kind !== 'array') continue;
    for (const notification of notifications.items) {
      if (
        notification.kind === 'object' &&
        literalString(notification.fields.provider) === provider
      ) {
        collectSecretNames(notification.fields.credentials, names);
      }
    }
  }
  return [...names];
}

function approvalDeliveries(
  workflow: CompiledWorkflowDefinition
): ReadonlyMap<BuiltInCommunicationProvider, ReadonlySet<string>> {
  const deliveries = new Map<BuiltInCommunicationProvider, Set<string>>();
  for (const node of workflowNodes(workflow)) {
    if (
      node.handler !== 'engine.approval-wait' ||
      node.inputs.kind !== 'object'
    )
      continue;
    const notifications = node.inputs.fields.notifications;
    if (notifications?.kind !== 'array') continue;
    for (const notification of notifications.items) {
      if (notification.kind !== 'object') continue;
      const provider = literalString(notification.fields.provider);
      const deliveryId = literalString(notification.fields.deliveryId);
      if (
        deliveryId === undefined ||
        (provider !== 'telegram' &&
          provider !== 'discord' &&
          provider !== 'whatsapp')
      ) {
        continue;
      }
      const providerDeliveries = deliveries.get(provider) ?? new Set<string>();
      providerDeliveries.add(deliveryId);
      deliveries.set(provider, providerDeliveries);
    }
  }
  return deliveries;
}

function providerRequirement(
  provider: BuiltInCommunicationProvider,
  workflows: readonly CompiledWorkflowDefinition[]
): ProviderActivationRequirementV1 {
  const triggerIds = new Set<string>();
  const notificationDeliveryIds = new Set<string>();
  const approvalDeliveryIds = new Set<string>();
  const credentialNames = new Set<string>();
  const inboundCredentials = new Set<string>();
  let messaging = false;

  for (const workflow of workflows) {
    for (const requirement of communicationRequirements(workflow)) {
      if (requirement.provider !== provider) continue;
      for (const triggerId of requirement.triggerIds) triggerIds.add(triggerId);
      for (const deliveryId of requirement.notificationDeliveryIds)
        notificationDeliveryIds.add(deliveryId);
      for (const credentialName of requirement.credentialNames)
        credentialNames.add(credentialName);
      messaging ||= requirement.messaging;
    }
    for (const deliveryId of approvalDeliveries(workflow).get(provider) ?? [])
      approvalDeliveryIds.add(deliveryId);
    for (const credentialName of inboundCredentialNames(provider, workflow))
      inboundCredentials.add(credentialName);
  }

  const sortedTriggerIds = [...triggerIds].sort();
  const sortedApprovalDeliveryIds = [...approvalDeliveryIds].sort();
  const sortedNotificationDeliveryIds = [...notificationDeliveryIds].sort();
  return {
    provider,
    used:
      sortedTriggerIds.length > 0 ||
      sortedNotificationDeliveryIds.length > 0 ||
      messaging,
    inbound:
      sortedTriggerIds.length > 0 || sortedApprovalDeliveryIds.length > 0,
    triggerIds: sortedTriggerIds,
    approvalDeliveryIds: sortedApprovalDeliveryIds,
    notificationDeliveryIds: sortedNotificationDeliveryIds,
    messaging,
    credentialNames: [...credentialNames].sort(),
    inboundCredentialNames: [...inboundCredentials].sort(),
  };
}

/**
 * Derives optional runtime infrastructure only from frozen compiled models.
 * The result is order-independent so loading the same workflow set in a
 * different CLI order cannot change which listeners or provider hosts start.
 */
export function runtimeActivationRequirements(
  workflows: readonly CompiledWorkflowDefinition[]
): RuntimeActivationRequirementsV1 {
  const triggerHandlers = [
    ...new Set(
      workflows.flatMap(workflow =>
        workflow.triggers.map(trigger => trigger.handler)
      )
    ),
  ].sort();
  const telegram = providerRequirement('telegram', workflows);
  const discord = providerRequirement('discord', workflows);
  const whatsapp = providerRequirement('whatsapp', workflows);
  const nodes = workflows.flatMap(workflow => workflowNodes(workflow));
  const lifecycleActions = workflows.flatMap(workflow =>
    'lifecycle' in workflow && workflow.lifecycle !== undefined
      ? workflow.lifecycle.hooks.flatMap(hook => hook.actions)
      : []
  );

  return {
    profile: RUNTIME_ACTIVATION_REQUIREMENTS_PROFILE,
    triggerHandlers,
    publicHttp:
      triggerHandlers.some(handler =>
        ['trigger.webhook', 'trigger.event'].includes(handler)
      ) || whatsapp.inbound,
    scriptExecution:
      nodes.some(node => node.handler === 'runtime.script') ||
      lifecycleActions.some(
        action => action.handler === 'runtime.lifecycle-script'
      ),
    runtimeModules: workflows.some(
      workflow =>
        'moduleRuntime' in workflow &&
        workflow.moduleRuntime !== undefined &&
        workflow.moduleRuntime.modules.length > 0
    ),
    providers: { telegram, discord, whatsapp },
  };
}
