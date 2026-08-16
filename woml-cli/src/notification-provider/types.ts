export const NOTIFICATION_PROVIDER_PROTOCOL =
  'woml.notification-provider-host' as const;
export const NOTIFICATION_PROVIDER_PROTOCOL_VERSION = 1 as const;
export const INFORMATIONAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION = 2 as const;
export const NOTIFICATION_PROVIDER_MAX_FRAME_BYTES = 1024 * 1024;

export type ApprovalDecision = 'approved' | 'rejected';
export type NotificationResolution =
  | 'approved'
  | 'rejected'
  | 'timeout_failed';

export interface SecretReference {
  readonly kind: 'secretReference';
  readonly name: string;
}

export interface NotificationCredentials {
  readonly botToken: SecretReference;
  readonly appToken: SecretReference;
}

export interface TelegramNotificationCredentials {
  readonly botToken: SecretReference;
}

export interface ApprovalMessage {
  readonly workflowId: string;
  readonly approvalName: string;
  readonly approvalDescription?: string;
  readonly expiresAt?: string;
}

export interface SlackProviderMessageIdentity {
  readonly workspaceId: string;
  readonly channelId: string;
  readonly messageId: string;
}

export interface TelegramProviderMessageIdentity {
  readonly provider: 'telegram';
  readonly accountId: string;
  readonly conversationId: string;
  readonly messageId: string;
}

export type ProviderMessageIdentity =
  | SlackProviderMessageIdentity
  | TelegramProviderMessageIdentity;

interface InvocationBase {
  readonly protocol: typeof NOTIFICATION_PROVIDER_PROTOCOL;
  readonly protocolVersion: typeof NOTIFICATION_PROVIDER_PROTOCOL_VERSION;
  readonly invocationId: string;
  readonly runId: string;
  readonly approvalId: string;
  readonly requestId: string;
  readonly deliveryId: string;
  readonly provider: 'slack' | 'telegram';
  readonly credentials:
    | NotificationCredentials
    | TelegramNotificationCredentials;
}

interface InformationalInvocationBase {
  readonly protocol: typeof NOTIFICATION_PROVIDER_PROTOCOL;
  readonly protocolVersion: typeof INFORMATIONAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION;
  readonly invocationId: string;
  readonly runId: string;
  readonly hookInvocationId: string;
  readonly actionId: string;
  readonly deliveryId: string;
  readonly provider: 'slack' | 'telegram';
  readonly destination: string;
  readonly idempotencyKey: string;
  readonly credentials:
    | NotificationCredentials
    | TelegramNotificationCredentials;
}

export interface DeliverMessage extends InvocationBase {
  readonly messageType: 'deliver';
  readonly destination: string;
  readonly idempotencyKey: string;
  readonly decisionCapability: string;
  readonly message: ApprovalMessage;
}

export interface UpdateMessage extends InvocationBase {
  readonly messageType: 'update';
  readonly updateId: string;
  readonly idempotencyKey: string;
  readonly providerMessage: ProviderMessageIdentity;
  readonly resolution: NotificationResolution;
}

export interface InformationalDeliverMessage extends InformationalInvocationBase {
  readonly messageType: 'deliver';
  readonly mode: 'informational';
  readonly message: string;
}

export type NotificationInvocation =
  | DeliverMessage
  | UpdateMessage
  | InformationalDeliverMessage;

export interface SlackTransportFailure {
  readonly kind:
    | 'secret_not_found'
    | 'provider_auth_failed'
    | 'destination_invalid'
    | 'rate_limited'
    | 'provider_unavailable'
    | 'delivery_ambiguous'
    | 'request_invalid'
    | 'host_crashed'
    | 'size_limit_exceeded'
    | 'update_failed';
  readonly code: string;
  readonly message: string;
  readonly retryable: boolean;
  readonly retryAfterMs?: number;
}

export type NotificationProviderFailure = SlackTransportFailure;

export type NotificationProviderOutcome =
  | {
      readonly kind: 'delivery_success';
      readonly providerMessage: ProviderMessageIdentity;
    }
  | { readonly kind: 'update_success' }
  | { readonly kind: 'failure'; readonly error: NotificationProviderFailure };

export interface CompletedMessage {
  readonly protocol: typeof NOTIFICATION_PROVIDER_PROTOCOL;
  readonly protocolVersion:
    | typeof NOTIFICATION_PROVIDER_PROTOCOL_VERSION
    | typeof INFORMATIONAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION;
  readonly messageType: 'completed';
  readonly invocationId: string;
  readonly outcome: NotificationProviderOutcome;
  readonly durationMs: number;
}

export interface ReadyMessage {
  readonly protocol: typeof NOTIFICATION_PROVIDER_PROTOCOL;
  readonly protocolVersion:
    | typeof NOTIFICATION_PROVIDER_PROTOCOL_VERSION
    | typeof INFORMATIONAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION;
  readonly messageType: 'ready';
  readonly hostInstanceId: string;
  readonly providers: readonly ('slack' | 'telegram')[];
}

export interface InteractionMessage {
  readonly protocol: typeof NOTIFICATION_PROVIDER_PROTOCOL;
  readonly protocolVersion: typeof NOTIFICATION_PROVIDER_PROTOCOL_VERSION;
  readonly messageType: 'interaction';
  readonly interactionId: string;
  readonly deliveryId: string;
  readonly provider: 'slack' | 'telegram';
  readonly decisionCapability: string;
  readonly decision: ApprovalDecision;
  readonly providerActorId: string;
  readonly occurredAt: string;
}

export type NotificationProviderOutbound =
  | ReadyMessage
  | CompletedMessage
  | InteractionMessage;

export interface ResolvedSlackCredentials {
  readonly botToken: string;
  readonly appToken: string;
}

export interface SlackDeliveryRequest {
  readonly invocation: DeliverMessage | InformationalDeliverMessage;
  readonly credentials: ResolvedSlackCredentials;
}

export interface SlackUpdateRequest {
  readonly invocation: UpdateMessage;
  readonly credentials: ResolvedSlackCredentials;
}
