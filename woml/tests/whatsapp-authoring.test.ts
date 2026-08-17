import { createHmac } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import { describe, expect, test } from 'bun:test';
import Ajv2020 from 'ajv/dist/2020';

import {
  compileWoml,
  generateWomlEditorDeclarations,
  parseWoml,
  verifyWhatsAppCallbackHandshake,
  verifyWhatsAppRawBodySignature,
  WOML_WHATSAPP_CALLBACK_PATH,
  WomlDiagnosticError,
} from '../src';

const repositoryRoot = resolve(import.meta.dir, '../..');
const schemaRoot = resolve(repositoryRoot, 'docs/schemas');
const fixtureRoot = resolve(import.meta.dir, 'fixtures/communication-providers');
const fixturePath = resolve(fixtureRoot, 'whatsapp.woml');

function document() {
  return parseWoml(readFileSync(fixturePath, 'utf8'), { file: fixturePath });
}

function workflow(
  content: string,
  body = 'return { ok: true };'
): string {
  return `<woml><workflow id="whatsapp-test" version="1.0.0">${content}<steps><step id="finish"><script>${body}</script></step></steps></workflow></woml>`;
}

function invalid(source: string): WomlDiagnosticError {
  try {
    compileWoml(parseWoml(source, { file: 'invalid-whatsapp.woml' }));
  } catch (error) {
    if (error instanceof WomlDiagnosticError) return error;
    throw error;
  }
  throw new Error('Expected invalid WhatsApp WOML.');
}

describe('WhatsApp authoring and callback contracts', () => {
  test('lowers trigger, approval, lifecycle, and messaging requirements into Model v15', () => {
    const compiled = compileWoml(document());
    expect(compiled.schemaVersion).toBe(15);
    if (compiled.schemaVersion !== 15) throw new Error('expected Model v15');
    expect(compiled.triggers[0]).toMatchObject({
      id: 'customerMessage',
      handler: 'trigger.whatsapp',
    });
    expect(compiled.communication.providers).toEqual([
      {
        provider: 'whatsapp',
        triggerIds: ['customerMessage'],
        notificationDeliveryIds: [
          'review:notify:0:recipient:0',
          'review:notify:0:recipient:1',
          'lifecycle:0:action:0:provider:0:recipient:0',
        ],
        messaging: true,
        credentialNames: [
          'WHATSAPP_ACCESS_TOKEN',
          'WHATSAPP_APP_SECRET',
          'WHATSAPP_VERIFY_TOKEN',
        ],
      },
    ]);

    const ajv = new Ajv2020({ strict: false, allErrors: true });
    for (const name of [
      ...Array.from({ length: 15 }, (_, index) => `compiled-workflow-model.v${index + 1}.schema.json`),
      'runtime-policy.v1.schema.json',
    ]) {
      ajv.addSchema(JSON.parse(readFileSync(resolve(schemaRoot, name), 'utf8')));
    }
    const validate = ajv.getSchema('https://woml.dev/schemas/compiled-workflow-model/v15')!;
    expect(validate(compiled), JSON.stringify(validate.errors, null, 2)).toBe(true);

  });

  test('rejects unsafe trigger, notification, and messaging shapes before activation', () => {
    expect(invalid(workflow(`<triggers><whatsapp id="message" events="status" phone-number-id="123456789012345" verify-token="{{secrets.WHATSAPP_VERIFY_TOKEN}}" app-secret="{{secrets.WHATSAPP_APP_SECRET}}" /></triggers>`)).diagnostic.code).toBe('WOML_WHATSAPP_TRIGGER_EVENT_INVALID');
    expect(invalid(workflow(`<triggers><manual id="start" /></triggers><lifecycle><on-complete><notify><whatsapp recipients="+1 555 123" access-token="{{secrets.WHATSAPP_ACCESS_TOKEN}}" phone-number-id="123456789012345" template="Bad-Name" language="english" message="done" /></notify></on-complete></lifecycle>`)).diagnostic.code).toBe('WOML_WHATSAPP_RECIPIENT_INVALID');
    expect(invalid(workflow(`<triggers><manual id="start" /></triggers><lifecycle><on-complete><notify><whatsapp recipients="15551234567" access-token="plaintext" phone-number-id="123456789012345" template="workflow_completed" language="en_US" message="done" /></notify></on-complete></lifecycle>`)).diagnostic.code).toBe('WOML_SECRET_LITERAL_FORBIDDEN');
    expect(invalid(workflow(`<triggers><manual id="start" /></triggers>`, `return services.whatsapp.send({ accessToken: secrets.WHATSAPP_ACCESS_TOKEN, phoneNumberId: '123456789012345', conversationId: '15551234567', text: 'unsafe free form' });`)).diagnostic.code).toBe('WOML_WHATSAPP_SEND_PROPERTY_REQUIRED');
  });

  test('pins callback handshake and exact raw-byte signature verification', () => {
    const contract = JSON.parse(
      readFileSync(resolve(fixtureRoot, 'whatsapp-callback.json'), 'utf8')
    );
    expect(WOML_WHATSAPP_CALLBACK_PATH).toBe(contract.route);
    expect(verifyWhatsAppCallbackHandshake(contract.handshake.query, 'synthetic-verify-token')).toEqual({ accepted: true, status: contract.handshake.expectedStatus, body: contract.handshake.expectedBody });
    expect(verifyWhatsAppCallbackHandshake({ mode: 'subscribe', verifyToken: 'wrong', challenge: 'challenge-42' }, 'verify-me')).toEqual({ accepted: false, status: 403, body: 'Callback verification rejected.' });

    const raw = Buffer.from(contract.signature.rawBodyBase64, 'base64');
    const secret = contract.signature.syntheticAppSecret;
    const signature = contract.signature.header;
    expect(signature).toBe(`sha256=${createHmac('sha256', secret).update(raw).digest('hex')}`);
    expect(verifyWhatsAppRawBodySignature(raw, signature, secret)).toBe(true);
    const reconstructed = new TextEncoder().encode('{"text":"héllo\\nمرحبا"}');
    expect(verifyWhatsAppRawBodySignature(reconstructed, signature, secret)).toBe(false);
  });

  test('publishes services.whatsapp.send editor declarations', () => {
    const declarations = generateWomlEditorDeclarations([]);
    expect(declarations).toContain('interface WomlWhatsAppSendRequest');
    expect(declarations).toContain('readonly whatsapp:');
    expect(declarations).toContain('Promise<WomlWhatsAppSendResult>');
  });
});
