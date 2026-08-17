import type {
  BuiltInCommunicationProvider,
  CommunicationMessagingAdapter,
  CommunicationNotificationAdapter,
  CommunicationTriggerAdapter,
} from './types';

type AnyNotificationAdapter = CommunicationNotificationAdapter<
  any,
  any,
  any,
  any
>;
type AnyMessagingAdapter = CommunicationMessagingAdapter<any, any, any, any>;

export type CommunicationProviderRegistration =
  | { readonly role: 'trigger'; readonly adapter: CommunicationTriggerAdapter }
  | { readonly role: 'notification'; readonly adapter: AnyNotificationAdapter }
  | { readonly role: 'messaging'; readonly adapter: AnyMessagingAdapter };

/**
 * Keeps provider roles separate. A trigger adapter cannot accidentally become
 * approval or messaging authority merely because it uses the same transport.
 */
export class CommunicationProviderRegistry {
  readonly #triggers = new Map<
    BuiltInCommunicationProvider,
    CommunicationTriggerAdapter
  >();
  readonly #notifications = new Map<
    BuiltInCommunicationProvider,
    AnyNotificationAdapter
  >();
  readonly #messaging = new Map<
    BuiltInCommunicationProvider,
    AnyMessagingAdapter
  >();

  register(registration: CommunicationProviderRegistration): void {
    const provider = registration.adapter.provider;
    if (registration.role === 'trigger') {
      this.#assertAvailable(this.#triggers, provider, registration.role);
      this.#triggers.set(provider, registration.adapter);
      return;
    }
    if (registration.role === 'notification') {
      this.#assertAvailable(this.#notifications, provider, registration.role);
      this.#notifications.set(provider, registration.adapter);
      return;
    }
    this.#assertAvailable(this.#messaging, provider, registration.role);
    this.#messaging.set(provider, registration.adapter);
  }

  triggerAdapters(): readonly CommunicationTriggerAdapter[] {
    return [...this.#triggers.values()];
  }

  notificationAdapter(
    provider: BuiltInCommunicationProvider
  ): AnyNotificationAdapter | undefined {
    return this.#notifications.get(provider);
  }

  messagingAdapter(
    provider: BuiltInCommunicationProvider
  ): AnyMessagingAdapter | undefined {
    return this.#messaging.get(provider);
  }

  providers(
    role: CommunicationProviderRegistration['role']
  ): readonly BuiltInCommunicationProvider[] {
    const entries =
      role === 'trigger'
        ? this.#triggers
        : role === 'notification'
          ? this.#notifications
          : this.#messaging;
    return [...entries.keys()];
  }

  #assertAvailable(
    entries: ReadonlyMap<BuiltInCommunicationProvider, unknown>,
    provider: BuiltInCommunicationProvider,
    role: CommunicationProviderRegistration['role']
  ): void {
    if (entries.has(provider)) {
      throw new Error(
        `Communication ${role} adapter "${provider}" is registered more than once.`
      );
    }
  }
}
