import { runNotificationProviderHost } from '../../src/notification-provider-host';
import {
  FakeSlackTransport,
  SlackTransportError,
  type InteractionMessage,
  type SlackDeliveryRequest,
  type SlackTransport,
  type SlackUpdateRequest,
} from '../../src/notification-provider';

process.env.WOML_SECRETS_PROVIDER = 'env';
process.env.WOML_SECRET_SLACK_BOT_TOKEN = 'xoxb-cross-provider-test';
process.env.WOML_SECRET_SLACK_APP_TOKEN = 'xapp-cross-provider-test';

await runNotificationProviderHost({
  adapter: 'fake',
  createTransport: emit => {
    const fake = new FakeSlackTransport({
      emit: message => emit(message as InteractionMessage),
      automaticDecision: 'approved',
    });
    let engineeringAttempts = 0;
    const transport: SlackTransport = {
      ensureConnection: (reference, value) =>
        fake.ensureConnection(reference, value),
      deliver: async (request: SlackDeliveryRequest) => {
        if (
          request.invocation.destination === '#engineering' &&
          ++engineeringAttempts === 1
        ) {
          throw new SlackTransportError({
            kind: 'rate_limited',
            code: 'WOML_SLACK_RATE_LIMITED',
            message: 'The sibling delivery is temporarily rate-limited.',
            retryable: true,
            retryAfterMs: 1,
          });
        }
        return fake.deliver(request);
      },
      update: (request: SlackUpdateRequest) => fake.update(request),
      close: () => fake.close(),
    };
    return transport;
  },
});
