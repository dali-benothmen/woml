import type { TriggerIngressAdmit, TriggerIngressOutcome } from '../rust-executor';

export type DiscordTriggerEventType = 'app-mention' | 'direct-message';

export interface DiscordTriggerRegistration {
  readonly workflowId: string;
  readonly definitionHash: string;
  readonly triggerId: string;
  readonly events: readonly DiscordTriggerEventType[];
  readonly channels: readonly string[];
  readonly credentialNames: { readonly botToken: string };
}

export interface DiscordTriggerPayload {
  readonly provider: 'discord';
  readonly event: DiscordTriggerEventType;
  readonly text: string;
  readonly senderId: string;
  readonly senderName?: string;
  readonly conversationId: string;
  readonly conversationType: 'direct' | 'group';
  readonly messageId: string;
  readonly replyToMessageId?: string;
  readonly occurredAt: string;
  readonly providerData: {
    readonly botId: string;
    readonly guildId?: string;
  };
}

export type SubmitDiscordTriggerIngress = (
  request: TriggerIngressAdmit
) => Promise<TriggerIngressOutcome>;

export interface DiscordBotIdentity {
  readonly botId: string;
  readonly username: string;
}

export interface DiscordMessageIdentity {
  readonly provider: 'discord';
  readonly accountId: string;
  readonly conversationId: string;
  readonly messageId: string;
}

export interface DiscordFailure {
  readonly kind:
    | 'secret_not_found'
    | 'provider_auth_failed'
    | 'permission_denied'
    | 'destination_invalid'
    | 'rate_limited'
    | 'provider_unavailable'
    | 'delivery_ambiguous'
    | 'request_invalid'
    | 'size_limit_exceeded'
    | 'update_failed';
  readonly code: string;
  readonly message: string;
  readonly retryable: boolean;
  readonly retryAfterMs?: number;
}

export interface DiscordMessageUpdate {
  readonly kind: 'message';
  readonly eventId: string;
  readonly payload: DiscordTriggerPayload;
}

export interface DiscordInteractionUpdate {
  readonly kind: 'interaction';
  readonly eventId: string;
  readonly interactionId: string;
  readonly interactionToken: string;
  readonly actorId: string;
  readonly decisionCapability: string;
  readonly decision: 'approved' | 'rejected';
}

export type DiscordNormalizedUpdate =
  | DiscordMessageUpdate
  | DiscordInteractionUpdate;

export type DiscordUpdateListener = (
  update: DiscordNormalizedUpdate
) => void | Promise<void>;
