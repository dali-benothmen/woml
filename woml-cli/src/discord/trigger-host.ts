import { randomUUID } from 'node:crypto';

import type { CommunicationTriggerAdapter } from '../communication-provider';
import { SecretStoreError, type SecretStore } from '../secrets';
import type { TriggerIngressAdmit } from '../rust-executor';
import { DiscordTransportError, SharedDiscordTransport } from './transport';
import type {
  DiscordInteractionUpdate,
  DiscordMessageUpdate,
  DiscordTriggerRegistration,
  SubmitDiscordTriggerIngress,
} from './types';

interface ActiveDiscordTrigger extends DiscordTriggerRegistration {
  readonly botId: string;
}

export interface DiscordTriggerHostOptions {
  readonly registrations: readonly DiscordTriggerRegistration[];
  readonly credentialNames?: readonly string[];
  readonly secretStore: SecretStore;
  readonly transport: SharedDiscordTransport;
  readonly submit: SubmitDiscordTriggerIngress;
  readonly resolveApproval?: (
    update: DiscordInteractionUpdate
  ) =>
    | void
    | 'accepted'
    | 'already-resolved'
    | 'expired'
    | Promise<void | 'accepted' | 'already-resolved' | 'expired'>;
  readonly diagnostic?: (code: string, message: string) => void;
}

export class DiscordTriggerHost implements CommunicationTriggerAdapter {
  readonly provider = 'discord' as const;
  readonly #options: DiscordTriggerHostOptions;
  readonly #listenerId = `discord_${randomUUID()}`;
  readonly #active: ActiveDiscordTrigger[] = [];
  readonly #unsubscribers: Array<() => void> = [];
  #started = false;
  #closed = false;

  constructor(options: DiscordTriggerHostOptions) {
    if (
      options.registrations.length === 0 &&
      (options.credentialNames?.length ?? 0) === 0
    ) {
      throw new Error(
        'DiscordTriggerHost requires a trigger or notification credential.'
      );
    }
    this.#options = options;
  }

  async start(): Promise<void> {
    if (this.#started || this.#closed) {
      throw new Error('Discord trigger host cannot be started twice.');
    }
    this.#started = true;
    const credentials = new Set([
      ...this.#options.registrations.map(
        item => item.credentialNames.botToken
      ),
      ...(this.#options.credentialNames ?? []),
    ]);
    try {
      for (const credentialName of credentials) {
        const token = await this.#options.secretStore.get(credentialName);
        if (token === undefined || token.length === 0) {
          throw new SecretStoreError(
            'WOML_SECRET_NOT_FOUND',
            `Missing required secret: ${credentialName}. Configure it with: woml secrets set ${credentialName}`
          );
        }
        this.#unsubscribers.push(
          this.#options.transport.subscribe(
            credentialName,
            `${this.#listenerId}:${credentialName}`,
            update => this.#onUpdate(credentialName, update)
          )
        );
        const identity = await this.#options.transport.ensureConnected(
          credentialName,
          token
        );
        for (const registration of this.#options.registrations) {
          if (registration.credentialNames.botToken === credentialName) {
            this.#active.push({ ...registration, botId: identity.botId });
          }
        }
        this.#options.diagnostic?.(
          'WOML_DISCORD_READY',
          `Discord bot @${identity.username} is ready.`
        );
      }
    } catch (error) {
      await this.close();
      throw error;
    }
  }

  async close(): Promise<void> {
    if (this.#closed) return;
    this.#closed = true;
    for (const unsubscribe of this.#unsubscribers.splice(0)) unsubscribe();
    this.#active.splice(0);
  }

  async #onUpdate(
    credentialName: string,
    update: DiscordMessageUpdate | DiscordInteractionUpdate
  ): Promise<void> {
    if (update.kind === 'interaction') {
      if (this.#options.resolveApproval === undefined) return;
      const resolution = await this.#options.resolveApproval(update);
      try {
        await this.#options.transport.acknowledgeInteraction(
          update,
          resolution === 'already-resolved'
            ? 'This approval was already resolved.'
            : resolution === 'expired'
              ? 'This approval has expired.'
              : update.decision === 'approved'
                ? 'Approved'
                : 'Rejected'
        );
      } catch (error) {
        const code = error instanceof DiscordTransportError
          ? error.failure.code
          : 'WOML_DISCORD_INTERACTION_ACK_FAILED';
        this.#options.diagnostic?.(
          code,
          'The approval decision was saved, but Discord could not acknowledge the button interaction.'
        );
      }
      return;
    }
    const candidates = this.#active.filter(registration => {
      if (
        registration.credentialNames.botToken !== credentialName ||
        registration.botId !== update.payload.providerData.botId ||
        !registration.events.includes(update.payload.event)
      ) return false;
      return update.payload.event !== 'app-mention' ||
        registration.channels.length === 0 ||
        registration.channels.includes(update.payload.conversationId);
    });
    for (const registration of candidates) {
      const request: TriggerIngressAdmit = {
        contract: 'woml.trigger-ingress',
        contractVersion: 1,
        messageType: 'admit',
        requestId: `request_discord_${randomUUID()}`,
        workflowId: registration.workflowId,
        definitionHash: registration.definitionHash,
        triggerId: registration.triggerId,
        triggerHandler: 'trigger.discord',
        sourceIdentity: `discord:${registration.botId}:${update.eventId}:${registration.workflowId}:${registration.triggerId}`,
        payload: { ...update.payload },
        receivedAt: new Date().toISOString(),
      };
      const outcome = await this.#options.submit(request);
      if (outcome.messageType === 'rejected') {
        throw new DiscordTransportError({
          kind: 'provider_unavailable',
          code: outcome.failure.code,
          message: outcome.failure.message,
          retryable: false,
        });
      }
      this.#options.diagnostic?.(
        outcome.duplicate
          ? 'WOML_DISCORD_TRIGGER_DUPLICATE'
          : 'WOML_DISCORD_TRIGGER_ACCEPTED',
        `${outcome.duplicate ? 'Recognized duplicate' : 'Accepted'} Discord ${update.payload.event} ${update.eventId} for trigger "${registration.triggerId}": ${outcome.runId}.`
      );
    }
  }
}

export function discordStartupError(error: unknown): {
  readonly code: string;
  readonly message: string;
} {
  if (error instanceof SecretStoreError) {
    return { code: error.code, message: error.message };
  }
  if (error instanceof DiscordTransportError) {
    return { code: error.failure.code, message: error.failure.message };
  }
  return {
    code: 'WOML_DISCORD_TRIGGER_UNAVAILABLE',
    message: 'The Discord Gateway host could not start safely.',
  };
}
