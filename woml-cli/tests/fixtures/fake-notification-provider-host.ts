import { runNotificationProviderHost } from '../../src/notification-provider-host';

process.env.WOML_SECRETS_PROVIDER = 'env';
process.env.WOML_SECRET_SLACK_BOT_TOKEN = 'xoxb-n4-secret-bot-value';
process.env.WOML_SECRET_SLACK_APP_TOKEN = 'xapp-n4-secret-app-value';
process.env.WOML_FAKE_SLACK_DECISION = 'approved';
process.env.WOML_FAKE_SLACK_ACTOR_ID = 'U12345678';

await runNotificationProviderHost({ adapter: 'fake' });
