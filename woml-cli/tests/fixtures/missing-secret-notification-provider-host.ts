import { runNotificationProviderHost } from '../../src/notification-provider-host';

process.env.WOML_SECRETS_PROVIDER = 'env';
delete process.env.WOML_SECRET_SLACK_BOT_TOKEN;
delete process.env.WOML_SECRET_SLACK_APP_TOKEN;
process.env.WOML_FAKE_SLACK_DECISION = 'approved';

await runNotificationProviderHost({ adapter: 'fake' });
