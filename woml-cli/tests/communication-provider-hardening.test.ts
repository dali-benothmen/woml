import { describe, expect, test } from 'bun:test';

import {
  COMMUNICATION_PROVIDER_MAX_SUBSCRIBERS,
} from '../src/communication-provider';
import { DiscordTransportError, SharedDiscordTransport } from '../src/discord';
import { SharedTelegramTransport, TelegramTransportError } from '../src/telegram';
import { SharedWhatsAppTransport, WhatsAppTransportError } from '../src/whatsapp';

const noop = async () => {};
const oversizedResponse = `${'x'.repeat(1_048_576)}x`;

describe('communication provider resource and privacy budgets', () => {
  test('bounds Telegram and Discord runtime subscribers deterministically', () => {
    const telegram = new SharedTelegramTransport();
    const discord = new SharedDiscordTransport();
    for (let index = 0; index < COMMUNICATION_PROVIDER_MAX_SUBSCRIBERS; index += 1) {
      telegram.subscribe('TELEGRAM_BOT_TOKEN', `telegram-${index}`, noop);
      discord.subscribe('DISCORD_BOT_TOKEN', `discord-${index}`, noop);
    }

    try {
      telegram.subscribe('TELEGRAM_BOT_TOKEN', 'telegram-overflow', noop);
      throw new Error('expected Telegram subscriber rejection');
    } catch (error) {
      expect(error).toBeInstanceOf(TelegramTransportError);
      expect((error as TelegramTransportError).failure).toMatchObject({
        kind: 'size_limit_exceeded',
        code: 'WOML_TELEGRAM_SUBSCRIBER_LIMIT',
        retryable: false,
      });
    }
    try {
      discord.subscribe('DISCORD_BOT_TOKEN', 'discord-overflow', noop);
      throw new Error('expected Discord subscriber rejection');
    } catch (error) {
      expect(error).toBeInstanceOf(DiscordTransportError);
      expect((error as DiscordTransportError).failure).toMatchObject({
        kind: 'size_limit_exceeded',
        code: 'WOML_DISCORD_SUBSCRIBER_LIMIT',
        retryable: false,
      });
    }
  });

  test('counts multibyte UTF-8 when bounding outbound provider requests', async () => {
    let requests = 0;
    const neverFetch = (async () => {
      requests += 1;
      throw new Error('the oversized request must not reach a provider');
    }) as unknown as typeof fetch;
    const message = '😀'.repeat(20_000);

    const telegram = new SharedTelegramTransport({ fetch: neverFetch });
    await expect(telegram.sendMessage({
      botToken: 'telegram-secret',
      accountId: '1',
      conversationId: '42',
      text: message,
    })).rejects.toMatchObject({
      failure: { code: 'WOML_TELEGRAM_REQUEST_TOO_LARGE' },
    });

    const discord = new SharedDiscordTransport({ fetch: neverFetch });
    await expect(discord.sendMessage({
      botToken: 'discord-secret',
      accountId: '123456789012345678',
      conversationId: '234567890123456789',
      text: message,
    })).rejects.toMatchObject({
      failure: { code: 'WOML_DISCORD_REQUEST_TOO_LARGE' },
    });

    const whatsapp = new SharedWhatsAppTransport({ fetch: neverFetch });
    await expect(whatsapp.sendTemplate({
      accessToken: 'whatsapp-secret',
      phoneNumberId: '123456789012345',
      conversationId: '15551234567',
      templateName: 'large_message',
      language: 'en_US',
      parameters: [message],
    })).rejects.toMatchObject({
      failure: { code: 'WOML_WHATSAPP_REQUEST_TOO_LARGE' },
    });
    expect(requests).toBe(0);
  });

  test('rejects oversized or control-character credentials before network use', async () => {
    let requests = 0;
    const neverFetch = (async () => {
      requests += 1;
      throw new Error('invalid credentials must not reach a provider');
    }) as unknown as typeof fetch;
    await expect(
      new SharedTelegramTransport({ fetch: neverFetch }).botIdentity('token\nheader')
    ).rejects.toMatchObject({
      failure: { code: 'WOML_TELEGRAM_CREDENTIAL_INVALID' },
    });
    await expect(
      new SharedDiscordTransport({ fetch: neverFetch }).botIdentity('x'.repeat(16_385))
    ).rejects.toMatchObject({
      failure: { code: 'WOML_DISCORD_CREDENTIAL_INVALID' },
    });
    await expect(
      new SharedWhatsAppTransport({ fetch: neverFetch }).sendTemplate({
        accessToken: 'token\rheader',
        phoneNumberId: '123456789012345',
        conversationId: '15551234567',
        templateName: 'notice',
        language: 'en_US',
        parameters: [],
      })
    ).rejects.toMatchObject({
      failure: { code: 'WOML_WHATSAPP_CREDENTIAL_INVALID' },
    });
    expect(requests).toBe(0);
  });

  test('rejects oversized provider responses without leaking credentials or bodies', async () => {
    const responseFetch = (async () => new Response(oversizedResponse)) as unknown as typeof fetch;
    const telegramSecret = 'telegram-secret-value';
    const discordSecret = 'discord-secret-value';
    const whatsappSecret = 'whatsapp-secret-value';

    const assertions: Array<Promise<unknown>> = [
      new SharedTelegramTransport({ fetch: responseFetch }).botIdentity(telegramSecret),
      new SharedDiscordTransport({ fetch: responseFetch }).botIdentity(discordSecret),
      new SharedWhatsAppTransport({ fetch: responseFetch }).sendTemplate({
        accessToken: whatsappSecret,
        phoneNumberId: '123456789012345',
        conversationId: '15551234567',
        templateName: 'notice',
        language: 'en_US',
        parameters: [],
      }),
    ];
    const expected = [
      ['WOML_TELEGRAM_RESPONSE_TOO_LARGE', telegramSecret],
      ['WOML_DISCORD_RESPONSE_TOO_LARGE', discordSecret],
      ['WOML_WHATSAPP_RESPONSE_TOO_LARGE', whatsappSecret],
    ] as const;

    for (let index = 0; index < assertions.length; index += 1) {
      try {
        await assertions[index];
        throw new Error('expected an oversized response rejection');
      } catch (error) {
        const serialized = JSON.stringify(error);
        expect(serialized).toContain(expected[index]![0]);
        expect(serialized).not.toContain(expected[index]![1]);
        expect(serialized).not.toContain(oversizedResponse.slice(0, 128));
      }
    }
  });

  test('uses bounded safe errors for malformed and revoked provider responses', async () => {
    const telegram = new SharedTelegramTransport({
      fetch: (async () => new Response('{not-json', { status: 502 })) as unknown as typeof fetch,
    });
    await expect(telegram.botIdentity('telegram-secret')).rejects.toMatchObject({
      failure: {
        code: 'WOML_TELEGRAM_UNAVAILABLE',
        retryable: true,
      },
    });

    const discord = new SharedDiscordTransport({
      fetch: (async () => Response.json(
        { message: 'raw provider permission detail' },
        { status: 403 }
      )) as unknown as typeof fetch,
    });
    try {
      await discord.botIdentity('discord-secret');
      throw new Error('expected revoked Discord permission');
    } catch (error) {
      expect(error).toBeInstanceOf(DiscordTransportError);
      expect((error as Error).message).not.toContain('raw provider permission detail');
    }

    const whatsapp = new SharedWhatsAppTransport({
      fetch: (async () => Response.json(
        { error: { code: 190, message: 'raw token detail' } },
        { status: 401 }
      )) as unknown as typeof fetch,
    });
    try {
      await whatsapp.sendTemplate({
        accessToken: 'whatsapp-secret',
        phoneNumberId: '123456789012345',
        conversationId: '15551234567',
        templateName: 'notice',
        language: 'en_US',
        parameters: [],
      });
      throw new Error('expected revoked WhatsApp token');
    } catch (error) {
      expect(error).toBeInstanceOf(WhatsAppTransportError);
      expect((error as Error).message).not.toContain('raw token detail');
    }
  });
});
