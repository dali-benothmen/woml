export const CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL =
  'woml.custom-notification-provider' as const;
export const CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL_VERSION = 1 as const;
export const CUSTOM_NOTIFICATION_PROVIDER_MAX_FRAME_BYTES = 1024 * 1024;

export type CustomNotificationDomain = 'approval' | 'informational';

export interface CustomNotificationAction {
  readonly url: string;
}

export interface CustomNotificationRequest {
  readonly kind: CustomNotificationDomain;
  readonly message: string;
  readonly deliveryId: string;
  readonly idempotencyKey: string;
  readonly actions?: {
    readonly approve: CustomNotificationAction;
    readonly reject: CustomNotificationAction;
  };
}

export interface CustomProviderExecuteMessage {
  readonly protocol: typeof CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL;
  readonly protocolVersion: typeof CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL_VERSION;
  readonly messageType: 'execute';
  readonly invocationId: string;
  readonly definitionDigest: string;
  readonly scriptArtifactId: string;
  readonly props: Readonly<Record<string, unknown>>;
  readonly notification: CustomNotificationRequest;
  readonly attempt: {
    readonly number: number;
    readonly max: number;
  };
  readonly limits: {
    readonly timeoutMs: number;
    readonly maxResultBytes: number;
  };
}

export interface CustomProviderCancelMessage {
  readonly protocol: typeof CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL;
  readonly protocolVersion: typeof CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL_VERSION;
  readonly messageType: 'cancel';
  readonly invocationId: string;
}

export type CustomProviderInbound =
  | CustomProviderExecuteMessage
  | CustomProviderCancelMessage;

export interface CustomProviderReadyMessage {
  readonly protocol: typeof CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL;
  readonly protocolVersion: typeof CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL_VERSION;
  readonly messageType: 'ready';
  readonly hostInstanceId: string;
}

export interface CustomProviderReceipt {
  readonly messageId?: string;
}

export type CustomProviderFailureKind =
  | 'script_threw'
  | 'timed_out'
  | 'cancelled'
  | 'non_json'
  | 'worker_crashed'
  | 'host_crashed'
  | 'context_too_large'
  | 'result_too_large'
  | 'delivery_ambiguous'
  | 'service_failed'
  | 'request_invalid';

export interface CustomProviderFailure {
  readonly kind: CustomProviderFailureKind;
  readonly code: string;
  readonly message: string;
  readonly retryable: boolean;
}

export type CustomProviderOutcome =
  | { readonly kind: 'succeeded'; readonly receipt: CustomProviderReceipt }
  | { readonly kind: 'failed'; readonly error: CustomProviderFailure };

export interface CustomProviderCompletedMessage {
  readonly protocol: typeof CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL;
  readonly protocolVersion: typeof CUSTOM_NOTIFICATION_PROVIDER_PROTOCOL_VERSION;
  readonly messageType: 'completed';
  readonly invocationId: string;
  readonly durationMs: number;
  readonly outcome: CustomProviderOutcome;
}

export type CustomProviderOutbound =
  | CustomProviderReadyMessage
  | CustomProviderCompletedMessage;
