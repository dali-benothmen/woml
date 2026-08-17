import type {
  ProviderMessageIdentity,
  SlackTransportFailure,
  SlackDeliveryRequest,
  SlackUpdateRequest,
} from './types';
import type { SecretStore } from '../secrets';
import { SecretStoreError } from '../secrets';

export class SlackTransportError extends Error {
  constructor(readonly failure: SlackTransportFailure) {
    super(failure.message);
    this.name = 'SlackTransportError';
  }
}

export interface SlackTransport {
  ensureConnection(
    appTokenReference: string,
    resolvedAppToken: string
  ): Promise<void>;
  deliver(request: SlackDeliveryRequest): Promise<ProviderMessageIdentity>;
  update(request: SlackUpdateRequest): Promise<void>;
  close(): Promise<void>;
}

export interface SlackCredentialReferences {
  readonly botToken: { readonly name: string };
  readonly appToken: { readonly name: string };
}

export async function resolveSlackCredentials(
  secretStore: SecretStore,
  references: SlackCredentialReferences
): Promise<{ readonly botToken: string; readonly appToken: string }> {
  const botToken = await secretStore.get(references.botToken.name);
  const appToken = await secretStore.get(references.appToken.name);
  const missing = [
    ...(botToken === undefined || botToken.length === 0
      ? [references.botToken.name]
      : []),
    ...(appToken === undefined || appToken.length === 0
      ? [references.appToken.name]
      : []),
  ];
  if (missing.length > 0) {
    throw new SecretStoreError(
      'WOML_SECRET_NOT_FOUND',
      `Missing required Slack credential${missing.length === 1 ? '' : 's'}: ${missing.join(', ')}.`
    );
  }
  return { botToken: botToken!, appToken: appToken! };
}
