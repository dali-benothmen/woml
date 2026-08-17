import { runNotificationProviderHost } from '../../src/notification-provider-host';

process.env.WOML_SECRETS_PROVIDER = 'env';
process.env.WOML_SECRET_SLACK_BOT_TOKEN = 'xoxb-n5-rate-limit-bot';
process.env.WOML_SECRET_SLACK_APP_TOKEN = 'xapp-n5-rate-limit-app';
process.env.WOML_FAKE_SLACK_DECISION = 'approved';
process.env.WOML_FAKE_SLACK_DELIVERY_FAILURES = '1';

await runNotificationProviderHost({ adapter: 'fake' });
