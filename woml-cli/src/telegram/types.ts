import type { TriggerIngressAdmit, TriggerIngressOutcome } from '../rust-executor';

export type TelegramTriggerEventType = 'message';

export interface TelegramTriggerRegistration {
  readonly workflowId: string;
  readonly definitionHash: string;
  readonly triggerId: string;
  readonly events: readonly TelegramTriggerEventType[];
  readonly credentialNames: { readonly botToken: string };
}

export interface TelegramTriggerPayload {
  readonly provider: 'telegram';
  readonly event: 'message';
  readonly text: string;
  readonly senderId: string;
  readonly senderName?: string;
  readonly conversationId: string;
  readonly conversationType: 'direct' | 'group' | 'channel';
  readonly messageId: string;
  readonly replyToMessageId?: string;
  readonly occurredAt: string;
  readonly providerData: { readonly botId: string };
}

export type SubmitTelegramTriggerIngress = (
  request: TriggerIngressAdmit
) => Promise<TriggerIngressOutcome>;

export interface TelegramBotIdentity {
  readonly botId: string;
  readonly username?: string;
}

export interface TelegramMessageIdentity {
  readonly provider: 'telegram';
  readonly accountId: string;
  readonly conversationId: string;
  readonly messageId: string;
}

export interface TelegramFailure {
  readonly kind:
    | 'secret_not_found'
    | 'provider_auth_failed'
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

export interface TelegramMessageUpdate {
  readonly kind: 'message';
  readonly updateId: number;
  readonly payload: TelegramTriggerPayload;
}

export interface TelegramCallbackUpdate {
  readonly kind: 'callback';
  readonly updateId: number;
  readonly callbackQueryId: string;
  readonly actorId: string;
  readonly decisionCapability: string;
  readonly decision: 'approved' | 'rejected';
}

export type TelegramNormalizedUpdate =
  | TelegramMessageUpdate
  | TelegramCallbackUpdate;

export type TelegramUpdateListener = (
  update: TelegramNormalizedUpdate
) => void | Promise<void>;
