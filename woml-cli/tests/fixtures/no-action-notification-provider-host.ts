import { runNotificationProviderHost } from '../../src/notification-provider-host';

process.env.WOML_SECRETS_PROVIDER = 'env';
process.env.WOML_SECRET_SLACK_BOT_TOKEN = 'xoxb-n4-timeout-bot-value';
process.env.WOML_SECRET_SLACK_APP_TOKEN = 'xapp-n4-timeout-app-value';
delete process.env.WOML_FAKE_SLACK_DECISION;

await runNotificationProviderHost();
