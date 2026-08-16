import type { SecretStore } from '../secrets';

export const BUILT_IN_COMMUNICATION_PROVIDERS = [
  'slack',
  'telegram',
  'discord',
  'whatsapp',
] as const;

export type BuiltInCommunicationProvider =
  (typeof BUILT_IN_COMMUNICATION_PROVIDERS)[number];

export interface ResolvedProviderCredentials<Credentials> {
  readonly credentials: Credentials;
  readonly secretValues: readonly string[];
}

export class CommunicationProviderAdapterError<Failure> extends Error {
  constructor(readonly failure: Failure) {
    super(
      typeof failure === 'object' &&
        failure !== null &&
        'message' in failure &&
        typeof failure.message === 'string'
        ? failure.message
        : 'The communication provider adapter failed.'
    );
    this.name = 'CommunicationProviderAdapterError';
  }
}

/**
 * Trusted adapter boundary used by the notification host. Wire envelopes stay
 * versioned separately; this interface only delegates provider-specific work.
 */
export interface CommunicationNotificationAdapter<
  Invocation,
  Credentials,
  MessageIdentity,
  Failure,
> {
  readonly provider: BuiltInCommunicationProvider;

  resolveCredentials(
    secretStore: SecretStore,
    invocation: Invocation
  ): Promise<ResolvedProviderCredentials<Credentials>>;

  prepare(invocation: Invocation, credentials: Credentials): Promise<void>;

  deliver(
    invocation: Invocation,
    credentials: Credentials
  ): Promise<MessageIdentity>;

  update(invocation: Invocation, credentials: Credentials): Promise<void>;

  validMessageIdentity(value: unknown): value is MessageIdentity;

  invalidMessageIdentityFailure(): Failure;

  safeFailure(error: unknown, resolvedValues: readonly string[]): Failure;

  close(): Promise<void>;
}

/** A provider trigger adapter owns transport, decoding, and acknowledgement. */
export interface CommunicationTriggerAdapter {
  readonly provider: BuiltInCommunicationProvider;
  start(): Promise<void>;
  close(): Promise<void>;
}

/** Provider-specific outbound messaging behind Rust capability supervision. */
export interface CommunicationMessagingAdapter<
  Request,
  Credentials,
  MessageIdentity,
  Failure,
> {
  readonly provider: BuiltInCommunicationProvider;

  resolveCredentials(
    secretStore: SecretStore,
    request: Request
  ): Promise<ResolvedProviderCredentials<Credentials>>;

  prepare(request: Request, credentials: Credentials): Promise<void>;

  send(request: Request, credentials: Credentials): Promise<MessageIdentity>;

  validMessageIdentity(value: unknown): value is MessageIdentity;

  safeFailure(error: unknown, resolvedValues: readonly string[]): Failure;

  close(): Promise<void>;
}
