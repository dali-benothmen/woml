import type { SecretStore } from './secrets';
import {
  ProviderResponseLimitError,
  providerCredentialWithinBudget,
  readProviderResponseBody,
} from './communication-provider/limits';

export type DoctorProvider = 'telegram' | 'discord' | 'whatsapp';
export type DoctorCheckStatus = 'pass' | 'warning' | 'fail';

export interface ProviderDoctorArguments {
  readonly provider: DoctorProvider;
  readonly json: boolean;
  readonly color: 'auto' | 'always' | 'never';
  readonly tokenSecret: string;
  readonly destination?: string;
  readonly appSecret: string;
  readonly verifyTokenSecret: string;
  readonly phoneNumberId?: string;
  readonly callbackUrl?: string;
}

export interface ProviderDoctorCheck {
  readonly id: string;
  readonly status: DoctorCheckStatus;
  readonly code: string;
  readonly message: string;
}

export interface ProviderDoctorResult {
  readonly profile: 'woml.provider-doctor/v1';
  readonly provider: DoctorProvider;
  readonly status: 'healthy' | 'degraded' | 'failed';
  readonly checks: readonly ProviderDoctorCheck[];
}

export interface ProviderDoctorDependencies {
  readonly secretStore: SecretStore;
  readonly fetch: typeof globalThis.fetch;
  readonly timeoutMs?: number;
}

const SAFE_RESPONSE_BYTES = 64 * 1024;

export function providerDoctorUsage(provider?: DoctorProvider): string {
  if (provider === 'telegram') {
    return 'Usage: woml telegram doctor [--token-secret <NAME>] [--destination <chatId>] [--json] [--color=auto|always|never]';
  }
  if (provider === 'discord') {
    return 'Usage: woml discord doctor [--token-secret <NAME>] [--destination <channelId>] [--json] [--color=auto|always|never]';
  }
  if (provider === 'whatsapp') {
    return 'Usage: woml whatsapp doctor [--access-token-secret <NAME>] [--app-secret <NAME>] [--verify-token-secret <NAME>] [--phone-number-id <id>] [--callback-url <https-url>] [--json] [--color=auto|always|never]';
  }
  return [
    providerDoctorUsage('telegram'),
    providerDoctorUsage('discord'),
    providerDoctorUsage('whatsapp'),
  ].join('\n');
}

function validSecretName(value: string): boolean {
  return /^[A-Z][A-Z0-9_]{0,127}$/.test(value);
}

export function parseProviderDoctorArguments(
  args: readonly string[]
): ProviderDoctorArguments {
  const provider = args[0];
  if (
    (provider !== 'telegram' && provider !== 'discord' && provider !== 'whatsapp') ||
    args[1] !== 'doctor'
  ) {
    throw new Error('WOML_CLI_ARGUMENTS_INVALID');
  }
  let json = false;
  let color: ProviderDoctorArguments['color'] = 'auto';
  let tokenSecret =
    provider === 'telegram'
      ? 'TELEGRAM_BOT_TOKEN'
      : provider === 'discord'
        ? 'DISCORD_BOT_TOKEN'
        : 'WHATSAPP_ACCESS_TOKEN';
  let appSecret = 'WHATSAPP_APP_SECRET';
  let verifyTokenSecret = 'WHATSAPP_VERIFY_TOKEN';
  let destination: string | undefined;
  let phoneNumberId: string | undefined;
  let callbackUrl: string | undefined;
  const seen = new Set<string>();
  for (let index = 2; index < args.length; index += 1) {
    const option = args[index]!;
    const optionIdentity = option.startsWith('--color=') ? '--color' : option;
    if (seen.has(optionIdentity)) throw new Error('WOML_CLI_ARGUMENTS_INVALID');
    seen.add(optionIdentity);
    if (option === '--json') {
      json = true;
      continue;
    }
    if (option.startsWith('--color=')) {
      const value = option.slice('--color='.length);
      if (value !== 'auto' && value !== 'always' && value !== 'never') {
        throw new Error('WOML_CLI_ARGUMENTS_INVALID');
      }
      color = value;
      continue;
    }
    const value = args[index + 1];
    if (value === undefined || value.startsWith('--')) {
      throw new Error('WOML_CLI_ARGUMENTS_INVALID');
    }
    index += 1;
    if (option === '--token-secret' && provider !== 'whatsapp') {
      tokenSecret = value;
    } else if (option === '--destination' && provider !== 'whatsapp') {
      destination = value;
    } else if (option === '--access-token-secret' && provider === 'whatsapp') {
      tokenSecret = value;
    } else if (option === '--app-secret' && provider === 'whatsapp') {
      appSecret = value;
    } else if (option === '--verify-token-secret' && provider === 'whatsapp') {
      verifyTokenSecret = value;
    } else if (option === '--phone-number-id' && provider === 'whatsapp') {
      phoneNumberId = value;
    } else if (option === '--callback-url' && provider === 'whatsapp') {
      callbackUrl = value;
    } else {
      throw new Error('WOML_CLI_ARGUMENTS_INVALID');
    }
  }
  for (const name of [
    tokenSecret,
    ...(provider === 'whatsapp' ? [appSecret, verifyTokenSecret] : []),
  ]) {
    if (!validSecretName(name)) throw new Error('WOML_CLI_ARGUMENTS_INVALID');
  }
  if (
    destination !== undefined &&
    (provider === 'telegram'
      ? !/^-?[0-9]{1,20}$/.test(destination)
      : !/^[0-9]{17,20}$/.test(destination))
  ) {
    throw new Error('WOML_CLI_ARGUMENTS_INVALID');
  }
  if (
    phoneNumberId !== undefined &&
    !/^[0-9]{6,32}$/.test(phoneNumberId)
  ) {
    throw new Error('WOML_CLI_ARGUMENTS_INVALID');
  }
  return {
    provider,
    json,
    color,
    tokenSecret,
    appSecret,
    verifyTokenSecret,
    ...(destination === undefined ? {} : { destination }),
    ...(phoneNumberId === undefined ? {} : { phoneNumberId }),
    ...(callbackUrl === undefined ? {} : { callbackUrl }),
  };
}

function check(
  id: string,
  status: DoctorCheckStatus,
  code: string,
  message: string
): ProviderDoctorCheck {
  return { id, status, code, message };
}

function safeStatus(checks: readonly ProviderDoctorCheck[]): ProviderDoctorResult['status'] {
  if (checks.some(item => item.status === 'fail')) return 'failed';
  if (checks.some(item => item.status === 'warning')) return 'degraded';
  return 'healthy';
}

async function requestJson(
  url: string,
  init: RequestInit,
  dependencies: ProviderDoctorDependencies
): Promise<{ readonly status: number; readonly ok: boolean; readonly body: unknown }> {
  const controller = new AbortController();
  const timeout = setTimeout(
    () => controller.abort(),
    dependencies.timeoutMs ?? 10_000
  );
  try {
    const response = await dependencies.fetch(url, {
      ...init,
      signal: controller.signal,
    });
    let text: string;
    try {
      text = await readProviderResponseBody(response, SAFE_RESPONSE_BYTES);
    } catch (error) {
      if (error instanceof ProviderResponseLimitError) {
        throw new Error('response_too_large');
      }
      throw error;
    }
    let body: unknown = {};
    if (text.length > 0) {
      try {
        body = JSON.parse(text);
      } catch {
        throw new Error('response_invalid');
      }
    }
    return { status: response.status, ok: response.ok, body };
  } finally {
    clearTimeout(timeout);
  }
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

async function requiredSecret(
  store: SecretStore,
  name: string,
  id: string,
  label: string,
  checks: ProviderDoctorCheck[]
): Promise<string | undefined> {
  try {
    const value = await store.get(name);
    if (value === undefined || value.length === 0) {
      checks.push(
        check(
          id,
          'fail',
          'WOML_SECRET_NOT_FOUND',
          `${label} is missing. Configure it with: woml secrets set ${name}`
        )
      );
      return undefined;
    }
    if (!providerCredentialWithinBudget(value)) {
      checks.push(
        check(
          id,
          'fail',
          'WOML_PROVIDER_CREDENTIAL_INVALID',
          `${label} has an invalid shape or exceeds the credential limit.`
        )
      );
      return undefined;
    }
    checks.push(check(id, 'pass', 'WOML_DOCTOR_SECRET_READY', `${label} is configured.`));
    return value;
  } catch {
    checks.push(
      check(
        id,
        'fail',
        'WOML_SECRET_UNAVAILABLE',
        `${label} could not be read from the configured secret store.`
      )
    );
    return undefined;
  }
}

async function telegramDoctor(
  args: ProviderDoctorArguments,
  dependencies: ProviderDoctorDependencies
): Promise<ProviderDoctorCheck[]> {
  const checks: ProviderDoctorCheck[] = [];
  const token = await requiredSecret(
    dependencies.secretStore,
    args.tokenSecret,
    'credentials',
    'Telegram bot token',
    checks
  );
  if (token === undefined) return checks;
  const call = (method: string, body: unknown) =>
    requestJson(
      `https://api.telegram.org/bot${token}/${method}`,
      {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(body),
      },
      dependencies
    );
  try {
    const identity = await call('getMe', {});
    const result = record(identity.body) ? identity.body.result : undefined;
    if (!identity.ok || !record(identity.body) || identity.body.ok !== true || !record(result) || result.is_bot !== true) {
      checks.push(check('authentication', 'fail', 'WOML_TELEGRAM_AUTH_FAILED', 'Telegram rejected the bot token or returned an invalid bot identity.'));
      return checks;
    }
    checks.push(check('authentication', 'pass', 'WOML_TELEGRAM_AUTH_READY', 'Telegram authenticated the configured bot.'));
  } catch (error) {
    checks.push(check('authentication', 'fail', error instanceof Error && error.message === 'response_too_large' ? 'WOML_TELEGRAM_RESPONSE_TOO_LARGE' : 'WOML_TELEGRAM_UNAVAILABLE', 'Telegram authentication could not be completed safely.'));
    return checks;
  }
  try {
    const webhook = await call('getWebhookInfo', {});
    const result = record(webhook.body) ? webhook.body.result : undefined;
    const active = record(result) && typeof result.url === 'string' && result.url.length > 0;
    checks.push(
      active
        ? check('polling', 'fail', 'WOML_TELEGRAM_POLLING_CONFLICT', 'A Telegram webhook is active. Remove it before WOML long polling can start.')
        : check('polling', 'pass', 'WOML_TELEGRAM_POLLING_READY', 'No Telegram webhook conflicts with WOML long polling.')
    );
  } catch {
    checks.push(check('polling', 'fail', 'WOML_TELEGRAM_UNAVAILABLE', 'Telegram webhook status could not be checked.'));
  }
  if (args.destination !== undefined) {
    try {
      const chat = await call('getChat', { chat_id: args.destination });
      checks.push(
        chat.ok && record(chat.body) && chat.body.ok === true
          ? check('destination', 'pass', 'WOML_TELEGRAM_DESTINATION_READY', 'Telegram can access the configured chat.')
          : check('destination', 'fail', 'WOML_TELEGRAM_DESTINATION_INVALID', 'Telegram cannot access the configured chat. Add the bot and confirm the numeric chat ID.')
      );
    } catch {
      checks.push(check('destination', 'fail', 'WOML_TELEGRAM_UNAVAILABLE', 'Telegram chat access could not be checked.'));
    }
  }
  return checks;
}

async function discordDoctor(
  args: ProviderDoctorArguments,
  dependencies: ProviderDoctorDependencies
): Promise<ProviderDoctorCheck[]> {
  const checks: ProviderDoctorCheck[] = [];
  const token = await requiredSecret(
    dependencies.secretStore,
    args.tokenSecret,
    'credentials',
    'Discord bot token',
    checks
  );
  if (token === undefined) return checks;
  const get = (path: string) =>
    requestJson(
      `https://discord.com/api/v10${path}`,
      { method: 'GET', headers: { authorization: `Bot ${token}` } },
      dependencies
    );
  try {
    const identity = await get('/users/@me');
    if (!identity.ok || !record(identity.body) || identity.body.bot !== true) {
      checks.push(check('authentication', 'fail', 'WOML_DISCORD_AUTH_FAILED', 'Discord rejected the bot token or returned an invalid bot identity.'));
      return checks;
    }
    checks.push(check('authentication', 'pass', 'WOML_DISCORD_AUTH_READY', 'Discord authenticated the configured bot.'));
  } catch (error) {
    checks.push(check('authentication', 'fail', error instanceof Error && error.message === 'response_too_large' ? 'WOML_DISCORD_RESPONSE_TOO_LARGE' : 'WOML_DISCORD_UNAVAILABLE', 'Discord authentication could not be completed safely.'));
    return checks;
  }
  try {
    const application = await get('/oauth2/applications/@me');
    if (!application.ok || !record(application.body)) {
      checks.push(check('application', 'fail', 'WOML_DISCORD_APPLICATION_UNAVAILABLE', 'Discord application metadata is unavailable for this bot token.'));
    } else {
      checks.push(check('application', 'pass', 'WOML_DISCORD_APPLICATION_READY', 'Discord application identity is available.'));
      const flags = typeof application.body.flags === 'number' ? application.body.flags : 0;
      const messageContent = (flags & (1 << 18)) !== 0 || (flags & (1 << 19)) !== 0;
      checks.push(
        messageContent
          ? check('message-content', 'pass', 'WOML_DISCORD_MESSAGE_CONTENT_READY', 'Discord Message Content intent is enabled for the application.')
          : check('message-content', 'warning', 'WOML_DISCORD_MESSAGE_CONTENT_UNCONFIRMED', 'Enable the Message Content intent when workflows need guild message text.')
      );
    }
  } catch {
    checks.push(check('application', 'fail', 'WOML_DISCORD_UNAVAILABLE', 'Discord application metadata could not be checked.'));
  }
  if (args.destination !== undefined) {
    try {
      const channel = await get(`/channels/${args.destination}`);
      checks.push(
        channel.ok && record(channel.body) && channel.body.id === args.destination
          ? check('destination', 'pass', 'WOML_DISCORD_DESTINATION_READY', 'Discord can access the configured channel.')
          : check('destination', 'fail', 'WOML_DISCORD_DESTINATION_INVALID', 'Discord cannot access the configured channel. Confirm the ID, membership, and permissions.')
      );
    } catch {
      checks.push(check('destination', 'fail', 'WOML_DISCORD_UNAVAILABLE', 'Discord channel access could not be checked.'));
    }
  }
  return checks;
}

async function whatsappDoctor(
  args: ProviderDoctorArguments,
  dependencies: ProviderDoctorDependencies
): Promise<ProviderDoctorCheck[]> {
  const checks: ProviderDoctorCheck[] = [];
  const accessToken = await requiredSecret(dependencies.secretStore, args.tokenSecret, 'access-token', 'WhatsApp access token', checks);
  await requiredSecret(dependencies.secretStore, args.appSecret, 'app-secret', 'WhatsApp app secret', checks);
  await requiredSecret(dependencies.secretStore, args.verifyTokenSecret, 'verify-token', 'WhatsApp callback verification token', checks);
  if (args.phoneNumberId === undefined) {
    checks.push(check('phone', 'fail', 'WOML_WHATSAPP_PHONE_REQUIRED', 'Provide the Meta Phone Number ID with --phone-number-id.'));
  } else if (accessToken !== undefined) {
    try {
      const response = await requestJson(
        `https://graph.facebook.com/v23.0/${args.phoneNumberId}?fields=id,display_phone_number,verified_name,quality_rating`,
        { method: 'GET', headers: { authorization: `Bearer ${accessToken}` } },
        dependencies
      );
      checks.push(
        response.ok && record(response.body) && response.body.id === args.phoneNumberId
          ? check('phone', 'pass', 'WOML_WHATSAPP_PHONE_READY', 'Meta authenticated the configured WhatsApp Phone Number ID.')
          : check('phone', 'fail', 'WOML_WHATSAPP_AUTH_FAILED', 'Meta rejected the access token or Phone Number ID.')
      );
    } catch (error) {
      checks.push(check('phone', 'fail', error instanceof Error && error.message === 'response_too_large' ? 'WOML_WHATSAPP_RESPONSE_TOO_LARGE' : 'WOML_WHATSAPP_UNAVAILABLE', 'WhatsApp phone identity could not be checked safely.'));
    }
  }
  if (args.callbackUrl === undefined) {
    checks.push(check('callback', 'warning', 'WOML_WHATSAPP_CALLBACK_UNCONFIRMED', 'Provide --callback-url to verify the production HTTPS callback shape.'));
  } else {
    let valid = false;
    try {
      const callback = new URL(args.callbackUrl);
      valid = callback.protocol === 'https:' && callback.pathname.endsWith('/callbacks/whatsapp');
    } catch {
      valid = false;
    }
    checks.push(
      valid
        ? check('callback', 'pass', 'WOML_WHATSAPP_CALLBACK_READY', 'The callback uses HTTPS and the WOML WhatsApp callback path.')
        : check('callback', 'fail', 'WOML_WHATSAPP_CALLBACK_INVALID', 'The callback must be a public HTTPS URL ending in /callbacks/whatsapp.')
    );
  }
  return checks;
}

export async function runProviderDoctor(
  args: ProviderDoctorArguments,
  dependencies: ProviderDoctorDependencies
): Promise<ProviderDoctorResult> {
  const checks =
    args.provider === 'telegram'
      ? await telegramDoctor(args, dependencies)
      : args.provider === 'discord'
        ? await discordDoctor(args, dependencies)
        : await whatsappDoctor(args, dependencies);
  return {
    profile: 'woml.provider-doctor/v1',
    provider: args.provider,
    status: safeStatus(checks),
    checks,
  };
}

export function formatProviderDoctor(
  result: ProviderDoctorResult,
  options: {
    readonly color?: ProviderDoctorArguments['color'];
    readonly isTTY?: boolean;
    readonly environment?: Readonly<Record<string, string | undefined>>;
  } = {}
): string {
  const environment = options.environment ?? process.env;
  const mode = options.color ?? 'auto';
  const colored =
    mode === 'always' ||
    (mode === 'auto' &&
      options.isTTY === true &&
      !Object.prototype.hasOwnProperty.call(environment, 'NO_COLOR') &&
      environment.TERM !== 'dumb');
  const paint = (value: string, code: number) =>
    colored ? `\u001b[${code}m${value}\u001b[0m` : value;
  const title = paint(`WOML ${result.provider.toUpperCase()} DOCTOR`, 36);
  const lines = [title, ''];
  for (const item of result.checks) {
    const icon = item.status === 'pass' ? '✓' : item.status === 'warning' ? '!' : '✗';
    const code = item.status === 'pass' ? 32 : item.status === 'warning' ? 33 : 31;
    lines.push(`${paint(icon, code)} ${item.message}`);
    if (item.status !== 'pass') lines.push(`  ${paint(item.code, code)}`);
  }
  const statusColor = result.status === 'healthy' ? 32 : result.status === 'degraded' ? 33 : 31;
  lines.push('', `Result: ${paint(result.status, statusColor)}`);
  return `${lines.join('\n')}\n`;
}
