import { createHmac, timingSafeEqual } from 'node:crypto';

/** Frozen public callback path. ACP7 attaches it to the production listener. */
export const WOML_WHATSAPP_CALLBACK_PATH = '/callbacks/whatsapp';
export const WOML_WHATSAPP_SIGNATURE_HEADER = 'x-hub-signature-256';

export interface WhatsAppVerificationQueryV1 {
  readonly mode?: string;
  readonly verifyToken?: string;
  readonly challenge?: string;
}

export interface WhatsAppVerificationResultV1 {
  readonly accepted: boolean;
  readonly status: 200 | 400 | 403;
  readonly body: string;
}

/**
 * Verifies the Meta webhook subscription handshake without resolving or
 * persisting any workflow data. The configured token remains capability data.
 */
export function verifyWhatsAppCallbackHandshake(
  query: WhatsAppVerificationQueryV1,
  configuredVerifyToken: string
): WhatsAppVerificationResultV1 {
  if (
    query.mode === undefined ||
    query.verifyToken === undefined ||
    query.challenge === undefined ||
    query.challenge.length === 0 ||
    query.challenge.length > 512
  ) {
    return { accepted: false, status: 400, body: 'Invalid callback verification request.' };
  }
  if (
    query.mode !== 'subscribe' ||
    configuredVerifyToken.length === 0 ||
    !constantTimeTextEqual(query.verifyToken, configuredVerifyToken)
  ) {
    return { accepted: false, status: 403, body: 'Callback verification rejected.' };
  }
  return { accepted: true, status: 200, body: query.challenge };
}

/**
 * Verifies X-Hub-Signature-256 against the exact HTTP bytes. Callers must not
 * parse, normalize, stringify, or otherwise reconstruct the body first.
 */
export function verifyWhatsAppRawBodySignature(
  rawBody: Uint8Array,
  signatureHeader: string | undefined,
  appSecret: string
): boolean {
  if (
    signatureHeader === undefined ||
    appSecret.length === 0 ||
    !/^sha256=[a-f0-9]{64}$/.test(signatureHeader)
  ) {
    return false;
  }
  const expected = createHmac('sha256', appSecret).update(rawBody).digest();
  const supplied = Buffer.from(signatureHeader.slice('sha256='.length), 'hex');
  return supplied.length === expected.length && timingSafeEqual(supplied, expected);
}

function constantTimeTextEqual(left: string, right: string): boolean {
  const leftBytes = Buffer.from(left, 'utf8');
  const rightBytes = Buffer.from(right, 'utf8');
  return (
    leftBytes.length === rightBytes.length && timingSafeEqual(leftBytes, rightBytes)
  );
}
