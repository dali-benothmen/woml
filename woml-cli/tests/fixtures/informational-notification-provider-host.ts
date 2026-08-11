import { runNotificationProviderHost } from '../../src/notification-provider-host';
import { INFORMATIONAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION } from '../../src/notification-provider';

await runNotificationProviderHost({
  adapter: 'fake',
  protocolVersion: INFORMATIONAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
});
