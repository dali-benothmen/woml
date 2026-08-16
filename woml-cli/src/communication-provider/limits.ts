export const COMMUNICATION_PROVIDER_MAX_CONNECTIONS = 64;
export const COMMUNICATION_PROVIDER_MAX_SUBSCRIBERS = 1_000;
export const COMMUNICATION_PROVIDER_MAX_API_RESPONSE_BYTES = 1_048_576;
export const COMMUNICATION_PROVIDER_MAX_API_REQUEST_BYTES = 65_536;
export const COMMUNICATION_PROVIDER_MAX_MESSAGE_BYTES = 40_000;
export const COMMUNICATION_PROVIDER_MAX_BATCH_ITEMS = 100;
export const COMMUNICATION_PROVIDER_MAX_CREDENTIAL_BYTES = 16_384;

export class ProviderResponseLimitError extends Error {
  constructor() {
    super('WOML_PROVIDER_RESPONSE_TOO_LARGE');
    this.name = 'ProviderResponseLimitError';
  }
}

export function providerCredentialWithinBudget(value: string): boolean {
  return (
    value.length > 0 &&
    Buffer.byteLength(value, 'utf8') <=
      COMMUNICATION_PROVIDER_MAX_CREDENTIAL_BYTES &&
    !/[\u0000-\u001f\u007f]/u.test(value)
  );
}

export async function readProviderResponseBody(
  response: Response,
  maximumBytes = COMMUNICATION_PROVIDER_MAX_API_RESPONSE_BYTES
): Promise<string> {
  const declared = Number(response.headers.get('content-length'));
  if (Number.isFinite(declared) && declared > maximumBytes) {
    throw new ProviderResponseLimitError();
  }
  if (response.body === null) return '';
  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  try {
    for (;;) {
      const chunk = await reader.read();
      if (chunk.done) break;
      total += chunk.value.byteLength;
      if (total > maximumBytes) {
        await reader.cancel();
        throw new ProviderResponseLimitError();
      }
      chunks.push(chunk.value);
    }
  } finally {
    reader.releaseLock();
  }
  const bytes = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return new TextDecoder().decode(bytes);
}

export function serializeProviderRequest(value: unknown): string {
  const body = JSON.stringify(value);
  if (
    Buffer.byteLength(body, 'utf8') >
    COMMUNICATION_PROVIDER_MAX_API_REQUEST_BYTES
  ) {
    throw new Error('WOML_PROVIDER_REQUEST_TOO_LARGE');
  }
  return body;
}
