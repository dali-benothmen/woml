export const NOTIFICATION_PROVIDER_PROTOCOL =
  'woml.notification-provider-host' as const;
export const NOTIFICATION_PROVIDER_PROTOCOL_VERSION = 1 as const;
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

export interface ApprovalMessage {
  readonly workflowId: string;
  readonly approvalName: string;
  readonly approvalDescription?: string;
  readonly expiresAt?: string;
}

export interface ProviderMessageIdentity {
  readonly workspaceId: string;
  readonly channelId: string;
  readonly messageId: string;
}

interface InvocationBase {
  readonly protocol: typeof NOTIFICATION_PROVIDER_PROTOCOL;
  readonly protocolVersion: typeof NOTIFICATION_PROVIDER_PROTOCOL_VERSION;
  readonly invocationId: string;
  readonly runId: string;
  readonly approvalId: string;
  readonly requestId: string;
  readonly deliveryId: string;
  readonly provider: 'slack';
  readonly credentials: NotificationCredentials;
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

export type NotificationInvocation = DeliverMessage | UpdateMessage;

export interface NotificationProviderFailure {
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

export type NotificationProviderOutcome =
  | {
      readonly kind: 'delivery_success';
      readonly providerMessage: ProviderMessageIdentity;
    }
  | { readonly kind: 'update_success' }
  | { readonly kind: 'failure'; readonly error: NotificationProviderFailure };

export interface CompletedMessage {
  readonly protocol: typeof NOTIFICATION_PROVIDER_PROTOCOL;
  readonly protocolVersion: typeof NOTIFICATION_PROVIDER_PROTOCOL_VERSION;
  readonly messageType: 'completed';
  readonly invocationId: string;
  readonly outcome: NotificationProviderOutcome;
  readonly durationMs: number;
}

export interface ReadyMessage {
  readonly protocol: typeof NOTIFICATION_PROVIDER_PROTOCOL;
  readonly protocolVersion: typeof NOTIFICATION_PROVIDER_PROTOCOL_VERSION;
  readonly messageType: 'ready';
  readonly hostInstanceId: string;
  readonly providers: readonly ['slack'];
}

export interface InteractionMessage {
  readonly protocol: typeof NOTIFICATION_PROVIDER_PROTOCOL;
  readonly protocolVersion: typeof NOTIFICATION_PROVIDER_PROTOCOL_VERSION;
  readonly messageType: 'interaction';
  readonly interactionId: string;
  readonly deliveryId: string;
  readonly provider: 'slack';
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
  readonly invocation: DeliverMessage;
  readonly credentials: ResolvedSlackCredentials;
}

export interface SlackUpdateRequest {
  readonly invocation: UpdateMessage;
  readonly credentials: ResolvedSlackCredentials;
}
