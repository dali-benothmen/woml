import { randomUUID } from 'node:crypto';

import type { CommunicationTriggerAdapter } from '../communication-provider';
import { SecretStoreError, type SecretStore } from '../secrets';
import type { TriggerIngressAdmit } from '../rust-executor';
import {
  SharedTelegramTransport,
  TelegramTransportError,
} from './transport';
import type {
  SubmitTelegramTriggerIngress,
  TelegramCallbackUpdate,
  TelegramMessageUpdate,
  TelegramTriggerRegistration,
} from './types';

interface ActiveTelegramTrigger extends TelegramTriggerRegistration {
  readonly botId: string;
}

export interface TelegramTriggerHostOptions {
  readonly registrations: readonly TelegramTriggerRegistration[];
  readonly credentialNames?: readonly string[];
  readonly secretStore: SecretStore;
  readonly transport: SharedTelegramTransport;
  readonly submit: SubmitTelegramTriggerIngress;
  readonly resolveApproval?: (
    update: TelegramCallbackUpdate
  ) =>
    | void
    | 'accepted'
    | 'already-resolved'
    | 'expired'
    | Promise<void | 'accepted' | 'already-resolved' | 'expired'>;
  readonly diagnostic?: (code: string, message: string) => void;
}

export class TelegramTriggerHost implements CommunicationTriggerAdapter {
  readonly provider = 'telegram' as const;
  readonly #options: TelegramTriggerHostOptions;
  readonly #listenerId = `telegram_${randomUUID()}`;
  readonly #active: ActiveTelegramTrigger[] = [];
  readonly #unsubscribers: Array<() => void> = [];
  #started = false;
  #closed = false;

  constructor(options: TelegramTriggerHostOptions) {
    if (
      options.registrations.length === 0 &&
      (options.credentialNames?.length ?? 0) === 0
    ) {
      throw new Error(
        'TelegramTriggerHost requires a trigger or notification credential.'
      );
    }
    this.#options = options;
  }

  async start(): Promise<void> {
    if (this.#started || this.#closed) {
      throw new Error('Telegram trigger host cannot be started twice.');
    }
    this.#started = true;
    const credentials = new Set([
      ...this.#options.registrations.map(item =>
        item.credentialNames.botToken
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
        const identity = await this.#options.transport.ensurePolling(
          credentialName,
          token
        );
        for (const registration of this.#options.registrations) {
          if (registration.credentialNames.botToken === credentialName) {
            this.#active.push({ ...registration, botId: identity.botId });
          }
        }
        this.#options.diagnostic?.(
          'WOML_TELEGRAM_READY',
          `Telegram bot${identity.username === undefined ? '' : ` @${identity.username}`} is ready.`
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
    update: TelegramMessageUpdate | TelegramCallbackUpdate
  ): Promise<void> {
    if (update.kind === 'callback') {
      if (this.#options.resolveApproval === undefined) return;
      const resolution = await this.#options.resolveApproval(update);
      try {
        await this.#options.transport.answerCallback(
          credentialName,
          update.callbackQueryId,
          resolution === 'already-resolved'
            ? 'This approval was already resolved.'
            : resolution === 'expired'
              ? 'This approval has expired.'
              : update.decision === 'approved'
                ? 'Approved'
                : 'Rejected'
        );
      } catch (error) {
        const code =
          error instanceof TelegramTransportError
            ? error.failure.code
            : 'WOML_TELEGRAM_CALLBACK_ACK_FAILED';
        this.#options.diagnostic?.(
          code,
          'The approval decision was saved, but Telegram could not acknowledge the button press.'
        );
      }
      return;
    }
    const candidates = this.#active.filter(
      registration =>
        registration.credentialNames.botToken === credentialName &&
        registration.botId === update.payload.providerData.botId &&
        registration.events.includes('message')
    );
    for (const registration of candidates) {
      const request: TriggerIngressAdmit = {
        contract: 'woml.trigger-ingress',
        contractVersion: 1,
        messageType: 'admit',
        requestId: `request_telegram_${randomUUID()}`,
        workflowId: registration.workflowId,
        definitionHash: registration.definitionHash,
        triggerId: registration.triggerId,
        triggerHandler: 'trigger.telegram',
        sourceIdentity: `telegram:${registration.botId}:${update.updateId}:${registration.workflowId}:${registration.triggerId}`,
        payload: { ...update.payload },
        receivedAt: new Date().toISOString(),
      };
      const outcome = await this.#options.submit(request);
      if (outcome.messageType === 'rejected') {
        throw new TelegramTransportError({
          kind: 'provider_unavailable',
          code: outcome.failure.code,
          message: outcome.failure.message,
          retryable: outcome.failure.retryable,
        });
      }
      this.#options.diagnostic?.(
        outcome.duplicate
          ? 'WOML_TELEGRAM_TRIGGER_DUPLICATE'
          : 'WOML_TELEGRAM_TRIGGER_ACCEPTED',
        `${outcome.duplicate ? 'Recognized duplicate' : 'Accepted'} Telegram message ${update.updateId} for trigger "${registration.triggerId}": ${outcome.runId}.`
      );
    }
  }
}

export function telegramStartupError(error: unknown): {
  readonly code: string;
  readonly message: string;
} {
  if (error instanceof SecretStoreError) {
    return { code: error.code, message: error.message };
  }
  if (error instanceof TelegramTransportError) {
    return {
      code: error.failure.code,
      message: error.failure.message,
    };
  }
  return {
    code: 'WOML_TELEGRAM_TRIGGER_UNAVAILABLE',
    message: 'The Telegram long-polling host could not start safely.',
  };
}
