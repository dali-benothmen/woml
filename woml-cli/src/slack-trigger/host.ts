import { randomUUID } from 'node:crypto';

import { SecretStoreError, type SecretStore } from '../secrets';
import {
  resolveSlackCredentials,
  SlackTransportError,
} from '../notification-provider/slack-transport';
import {
  type SharedSlackTransport,
  type SlackEnvelope,
} from '../notification-provider/shared-slack';
import type { TriggerIngressAdmit } from '../rust-executor';
import {
  SLACK_TRIGGER_PROTOCOL,
  SLACK_TRIGGER_PROTOCOL_VERSION,
  type SlackTriggerPayload,
  type SlackTriggerProtocolMessage,
  type SlackTriggerRegistration,
  type SubmitSlackTriggerIngress,
} from './types';

interface ActiveSlackTrigger extends SlackTriggerRegistration {
  readonly teamId: string;
  readonly botUserId: string;
  readonly acceptedChannelIds: ReadonlySet<string>;
}

export interface SlackTriggerHostOptions {
  readonly registrations: readonly SlackTriggerRegistration[];
  readonly secretStore: SecretStore;
  readonly transport: SharedSlackTransport;
  readonly submit: SubmitSlackTriggerIngress;
  readonly emit?: (
    message: SlackTriggerProtocolMessage
  ) => void | Promise<void>;
  readonly diagnostic?: (message: string) => void;
  readonly now?: () => Date;
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function validId(value: unknown, prefix: string): value is string {
  return (
    typeof value === 'string' &&
    value.length >= 3 &&
    value.length <= 128 &&
    value.startsWith(prefix) &&
    [...value.slice(1)].every(character => /[A-Z0-9]/.test(character))
  );
}

function validTimestamp(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    /^[0-9]+\.[0-9]+$/.test(value) &&
    value.length <= 64
  );
}

function validChannelId(value: unknown, directMessage: boolean): value is string {
  return directMessage
    ? validId(value, 'D')
    : validId(value, 'C') || validId(value, 'G');
}

function decodedPayload(
  body: Readonly<Record<string, unknown>>
):
  | {
      readonly kind: 'event';
      readonly eventId: string;
      readonly payload: SlackTriggerPayload;
    }
  | { readonly kind: 'ignored'; readonly reason: string }
  | { readonly kind: 'invalid'; readonly tooLarge?: boolean } {
  if (body.type !== 'events_api' || !record(body.payload)) {
    return { kind: 'ignored', reason: 'not a Slack Events API envelope' };
  }
  const outer = body.payload;
  if (
    outer.type !== 'event_callback' ||
    typeof outer.event_id !== 'string' ||
    outer.event_id.length === 0 ||
    outer.event_id.length > 256 ||
    !validId(outer.team_id, 'T') ||
    !record(outer.event)
  ) {
    return { kind: 'invalid' };
  }
  const event = outer.event;
  if (
    event.bot_id !== undefined ||
    event.bot_profile !== undefined ||
    event.subtype !== undefined
  ) {
    return { kind: 'ignored', reason: 'bot or unsupported message subtype' };
  }
  const type =
    event.type === 'app_mention'
      ? 'app-mention'
      : event.type === 'message' && event.channel_type === 'im'
        ? 'direct-message'
        : undefined;
  if (type === undefined) {
    return { kind: 'ignored', reason: 'event type is not enabled by WOML' };
  }
  if (
    typeof event.text !== 'string' ||
    !validId(event.user, 'U') ||
    !validChannelId(event.channel, type === 'direct-message') ||
    !validTimestamp(event.ts) ||
    (event.thread_ts !== undefined && !validTimestamp(event.thread_ts))
  ) {
    return { kind: 'invalid' };
  }
  if (Buffer.byteLength(event.text, 'utf8') > 40_000) {
    return { kind: 'invalid', tooLarge: true };
  }
  return {
    kind: 'event',
    eventId: outer.event_id,
    payload: {
      type,
      text: event.text,
      userId: event.user,
      channelId: event.channel,
      messageTs: event.ts,
      threadTs: event.thread_ts ?? event.ts,
      teamId: outer.team_id,
    },
  };
}

export class SlackTriggerHost {
  readonly #options: SlackTriggerHostOptions;
  readonly #listenerId = `trigger_${randomUUID()}`;
  readonly #unsubscribers: Array<() => void> = [];
  readonly #subscribedAppTokens = new Set<string>();
  readonly #active: ActiveSlackTrigger[] = [];
  #started = false;
  #closed = false;

  constructor(options: SlackTriggerHostOptions) {
    if (options.registrations.length === 0) {
      throw new Error('SlackTriggerHost requires at least one registration.');
    }
    this.#options = options;
  }

  async start(): Promise<void> {
    if (this.#started || this.#closed) {
      throw new Error('Slack trigger host cannot be started twice.');
    }
    this.#started = true;
    try {
      for (const registration of this.#options.registrations) {
        const credentials = await resolveSlackCredentials(
          this.#options.secretStore,
          {
            botToken: { name: registration.credentialNames.botToken },
            appToken: { name: registration.credentialNames.appToken },
          }
        );
        const identity = await this.#options.transport.botIdentity(
          registration.credentialNames.botToken,
          credentials.botToken
        );
        const acceptedChannelIds = new Set<string>();
        for (const channel of registration.channels) {
          acceptedChannelIds.add(
            await this.#options.transport.resolveDestination(
              channel.startsWith('#') || /^[CGD]/.test(channel)
                ? channel
                : `#${channel}`,
              registration.credentialNames.botToken,
              credentials.botToken
            )
          );
        }
        this.#active.push({
          ...registration,
          teamId: identity.teamId,
          botUserId: identity.userId,
          acceptedChannelIds,
        });
        if (!this.#subscribedAppTokens.has(registration.credentialNames.appToken)) {
          this.#unsubscribers.push(this.#options.transport.subscribe(
            registration.credentialNames.appToken,
            this.#listenerId,
            envelope => this.#onEnvelope(envelope)
          ));
          this.#subscribedAppTokens.add(
            registration.credentialNames.appToken
          );
        }
        await this.#emit({
          protocol: SLACK_TRIGGER_PROTOCOL,
          protocolVersion: SLACK_TRIGGER_PROTOCOL_VERSION,
          messageType: 'connection',
          workspaceId: identity.teamId,
          state: 'connecting',
          occurredAt: this.#now(),
        });
        await this.#options.transport.ensureConnection(
          registration.credentialNames.appToken,
          credentials.appToken
        );
        await this.#emit({
          protocol: SLACK_TRIGGER_PROTOCOL,
          protocolVersion: SLACK_TRIGGER_PROTOCOL_VERSION,
          messageType: 'connection',
          workspaceId: identity.teamId,
          state: 'ready',
          occurredAt: this.#now(),
        });
      }
      await this.#emit({
        protocol: SLACK_TRIGGER_PROTOCOL,
        protocolVersion: SLACK_TRIGGER_PROTOCOL_VERSION,
        messageType: 'ready',
      });
    } catch (error) {
      this.#cleanup();
      throw error;
    }
  }

  async close(): Promise<void> {
    if (this.#closed) return;
    this.#closed = true;
    const workspaces = new Set(this.#active.map(item => item.teamId));
    this.#cleanup();
    for (const workspaceId of workspaces) {
      await this.#emit({
        protocol: SLACK_TRIGGER_PROTOCOL,
        protocolVersion: SLACK_TRIGGER_PROTOCOL_VERSION,
        messageType: 'connection',
        workspaceId,
        state: 'stopped',
        occurredAt: this.#now(),
      });
    }
  }

  async #onEnvelope(envelope: SlackEnvelope): Promise<void> {
    const decoded = decodedPayload(envelope.body);
    if (decoded.kind === 'ignored') {
      if (envelope.body.type === 'events_api') envelope.acknowledge();
      return;
    }
    if (decoded.kind === 'invalid') {
      envelope.acknowledge();
      await this.#failure(
        envelope.envelopeId,
        decoded.tooLarge
          ? 'WOML_SLACK_TRIGGER_EVENT_TOO_LARGE'
          : 'WOML_SLACK_TRIGGER_EVENT_INVALID',
        decoded.tooLarge
          ? 'Slack delivered a message larger than the trigger protocol limit.'
          : 'Slack delivered a malformed trigger event.',
        false
      );
      return;
    }
    if (envelope.envelopeId === undefined) {
      await this.#failure(
        undefined,
        'WOML_SLACK_TRIGGER_EVENT_INVALID',
        'Slack delivered an event without an envelope identity.',
        false
      );
      return;
    }

    const candidates = this.#active.filter(registration => {
      if (
        !envelope.appTokenReferences.includes(
          registration.credentialNames.appToken
        ) ||
        registration.teamId !== decoded.payload.teamId ||
        registration.botUserId === decoded.payload.userId ||
        !registration.events.includes(decoded.payload.type)
      ) {
        return false;
      }
      return (
        decoded.payload.type === 'direct-message' ||
        registration.acceptedChannelIds.size === 0 ||
        registration.acceptedChannelIds.has(decoded.payload.channelId)
      );
    });
    if (candidates.length === 0) {
      envelope.acknowledge();
      this.#options.diagnostic?.(
        `Ignored Slack ${decoded.payload.type} ${decoded.eventId}: no WOML trigger matched its workspace, event, or channel filters.`
      );
      return;
    }

    const results = await Promise.all(
      candidates.map(async registration => {
        const occurredAt = this.#now();
        await this.#emit({
          protocol: SLACK_TRIGGER_PROTOCOL,
          protocolVersion: SLACK_TRIGGER_PROTOCOL_VERSION,
          messageType: 'event',
          envelopeId: envelope.envelopeId!,
          eventId: decoded.eventId,
          workspaceId: decoded.payload.teamId,
          workflowId: registration.workflowId,
          definitionHash: registration.definitionHash,
          triggerId: registration.triggerId,
          credentialNames: registration.credentialNames,
          payload: { ...decoded.payload },
          occurredAt,
        });
        const ingress: TriggerIngressAdmit = {
          contract: 'woml.trigger-ingress',
          contractVersion: 1,
          messageType: 'admit',
          requestId: `request_slack_${randomUUID()}`,
          workflowId: registration.workflowId,
          definitionHash: registration.definitionHash,
          triggerId: registration.triggerId,
          triggerHandler: 'trigger.slack',
          sourceIdentity: `slack:${decoded.payload.teamId}:${decoded.eventId}:${registration.workflowId}:${registration.triggerId}`,
          payload: { ...decoded.payload },
          receivedAt: occurredAt,
        };
        return await this.#options.submit(ingress);
      })
    ).catch(async () => {
      await this.#failure(
        envelope.envelopeId,
        'WOML_TRIGGER_UNAVAILABLE',
        'Rust trigger ingress is unavailable; Slack may redeliver this event.',
        true
      );
      return undefined;
    });
    if (results === undefined) return;
    const rejected = results.find(result => result.messageType === 'rejected');
    if (rejected?.messageType === 'rejected') {
      await this.#failure(
        envelope.envelopeId,
        rejected.failure.code === 'WOML_POLICY_QUEUE_FULL'
          ? 'WOML_POLICY_QUEUE_FULL'
          : 'WOML_TRIGGER_UNAVAILABLE',
        rejected.failure.message,
        rejected.failure.retryable
      );
      return;
    }

    envelope.acknowledge();
    for (const result of results) {
      if (result.messageType !== 'accepted') continue;
      await this.#emit({
        protocol: SLACK_TRIGGER_PROTOCOL,
        protocolVersion: SLACK_TRIGGER_PROTOCOL_VERSION,
        messageType: 'acknowledge',
        envelopeId: envelope.envelopeId,
        runId: result.runId,
        duplicate: result.duplicate,
      });
    }
  }

  async #failure(
    envelopeId: string | undefined,
    code: Extract<SlackTriggerProtocolMessage, { messageType: 'failure' }>['code'],
    message: string,
    retryable: boolean
  ): Promise<void> {
    await this.#emit({
      protocol: SLACK_TRIGGER_PROTOCOL,
      protocolVersion: SLACK_TRIGGER_PROTOCOL_VERSION,
      messageType: 'failure',
      ...(envelopeId === undefined ? {} : { envelopeId }),
      code,
      message,
      retryable,
    });
  }

  async #emit(message: SlackTriggerProtocolMessage): Promise<void> {
    await this.#options.emit?.(message);
  }

  #now(): string {
    return (this.#options.now?.() ?? new Date()).toISOString();
  }

  #cleanup(): void {
    for (const unsubscribe of this.#unsubscribers.splice(0)) unsubscribe();
    this.#subscribedAppTokens.clear();
    this.#active.splice(0);
  }
}

export function slackTriggerStartupError(error: unknown): {
  readonly code: string;
  readonly message: string;
} {
  if (error instanceof SecretStoreError) {
    return { code: error.code, message: error.message };
  }
  if (error instanceof SlackTransportError) {
    return {
      code:
        error.failure.code === 'WOML_SLACK_PERMISSION_DENIED'
          ? 'WOML_SLACK_TRIGGER_SCOPE_MISSING'
          : 'WOML_SLACK_TRIGGER_UNAVAILABLE',
      message: error.failure.message,
    };
  }
  return {
    code: 'WOML_SLACK_TRIGGER_UNAVAILABLE',
    message: 'The Slack trigger connection could not be started safely.',
  };
}
