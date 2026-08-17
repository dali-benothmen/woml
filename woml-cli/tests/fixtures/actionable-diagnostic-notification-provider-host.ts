import { runNotificationProviderHost } from '../../src/notification-provider-host';
import {
  FakeSlackTransport,
  SlackTransportError,
  type SlackTransport,
} from '../../src/notification-provider';

process.env.WOML_SECRETS_PROVIDER = 'env';
process.env.WOML_SECRET_SLACK_BOT_TOKEN =
  'xoxb-n61-secret-that-must-not-appear';
process.env.WOML_SECRET_SLACK_APP_TOKEN =
  'xapp-n61-secret-that-must-not-appear';

await runNotificationProviderHost({
  createTransport: emit => {
    const fake = new FakeSlackTransport({
      emit,
      automaticDecision: 'approved',
      automaticActorId: 'U12345678',
    });
    const transport: SlackTransport = {
      ensureConnection: (reference, token) =>
        fake.ensureConnection(reference, token),
      async deliver(request) {
        if (request.invocation.destination === '#engineering') {
          throw new SlackTransportError({
            kind: 'provider_auth_failed',
            code: 'WOML_SLACK_PERMISSION_DENIED',
            message:
              'Slack operation conversations.list needs additional app permissions. Missing scopes: channels:read. Granted scopes: chat:write. Add the missing Bot Token Scopes and reinstall the Slack app to the workspace.',
            retryable: false,
          });
        }
        return await fake.deliver(request);
      },
      update: request => fake.update(request),
      close: () => fake.close(),
    };
    return transport;
  },
});
