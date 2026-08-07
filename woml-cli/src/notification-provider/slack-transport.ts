import type {
  NotificationProviderFailure,
  ProviderMessageIdentity,
  SlackDeliveryRequest,
  SlackUpdateRequest,
} from './types';

export class SlackTransportError extends Error {
  constructor(readonly failure: NotificationProviderFailure) {
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
