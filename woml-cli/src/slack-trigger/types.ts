import type { TriggerIngressAdmit, TriggerIngressOutcome } from '../rust-executor';

export const SLACK_TRIGGER_PROTOCOL = 'woml.slack-trigger' as const;
export const SLACK_TRIGGER_PROTOCOL_VERSION = 1 as const;

export type SlackTriggerEventType = 'app-mention' | 'direct-message';

export interface SlackTriggerPayload {
  readonly type: SlackTriggerEventType;
  readonly text: string;
  readonly userId: string;
  readonly channelId: string;
  readonly messageTs: string;
  readonly threadTs: string;
  readonly teamId: string;
}

export interface SlackTriggerRegistration {
  readonly workflowId: string;
  readonly definitionHash: string;
  readonly triggerId: string;
  readonly events: readonly SlackTriggerEventType[];
  readonly channels: readonly string[];
  readonly credentialNames: {
    readonly botToken: string;
    readonly appToken: string;
  };
}

export type SlackTriggerProtocolMessage =
  | {
      readonly protocol: typeof SLACK_TRIGGER_PROTOCOL;
      readonly protocolVersion: typeof SLACK_TRIGGER_PROTOCOL_VERSION;
      readonly messageType: 'ready';
    }
  | {
      readonly protocol: typeof SLACK_TRIGGER_PROTOCOL;
      readonly protocolVersion: typeof SLACK_TRIGGER_PROTOCOL_VERSION;
      readonly messageType: 'connection';
      readonly workspaceId: string;
      readonly state: 'connecting' | 'ready' | 'reconnecting' | 'stopped';
      readonly retryAt?: string;
      readonly occurredAt: string;
    }
  | {
      readonly protocol: typeof SLACK_TRIGGER_PROTOCOL;
      readonly protocolVersion: typeof SLACK_TRIGGER_PROTOCOL_VERSION;
      readonly messageType: 'event';
      readonly envelopeId: string;
      readonly eventId: string;
      readonly workspaceId: string;
      readonly workflowId: string;
      readonly definitionHash: string;
      readonly triggerId: string;
      readonly credentialNames: SlackTriggerRegistration['credentialNames'];
      readonly payload: SlackTriggerPayload;
      readonly occurredAt: string;
    }
  | {
      readonly protocol: typeof SLACK_TRIGGER_PROTOCOL;
      readonly protocolVersion: typeof SLACK_TRIGGER_PROTOCOL_VERSION;
      readonly messageType: 'acknowledge';
      readonly envelopeId: string;
      readonly runId: string;
      readonly duplicate: boolean;
    }
  | {
      readonly protocol: typeof SLACK_TRIGGER_PROTOCOL;
      readonly protocolVersion: typeof SLACK_TRIGGER_PROTOCOL_VERSION;
      readonly messageType: 'failure';
      readonly envelopeId?: string;
      readonly code:
        | 'WOML_SLACK_TRIGGER_SCOPE_MISSING'
        | 'WOML_SLACK_TRIGGER_UNAVAILABLE'
        | 'WOML_SLACK_TRIGGER_EVENT_INVALID'
        | 'WOML_SLACK_TRIGGER_EVENT_TOO_LARGE'
        | 'WOML_POLICY_QUEUE_FULL'
        | 'WOML_TRIGGER_UNAVAILABLE';
      readonly message: string;
      readonly retryable: boolean;
    };

export type SubmitSlackTriggerIngress = (
  request: TriggerIngressAdmit
) => Promise<TriggerIngressOutcome>;
