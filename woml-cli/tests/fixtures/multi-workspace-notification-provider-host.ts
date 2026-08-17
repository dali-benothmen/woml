import { runNotificationProviderHost } from '../../src/notification-provider-host';

process.env.WOML_SECRETS_PROVIDER = 'env';
process.env.WOML_SECRET_SLACK_BOT_TOKEN = 'xoxb-n6-primary-bot-value';
process.env.WOML_SECRET_SLACK_APP_TOKEN = 'xapp-n6-primary-app-value';
process.env.WOML_SECRET_SECOND_SLACK_BOT_TOKEN =
  'xoxb-n6-secondary-bot-value';
process.env.WOML_SECRET_SECOND_SLACK_APP_TOKEN =
  'xapp-n6-secondary-app-value';
process.env.WOML_FAKE_SLACK_DECISION = 'approved';
process.env.WOML_FAKE_SLACK_ACTOR_ID = 'U12345678';

await runNotificationProviderHost({ adapter: 'fake' });
