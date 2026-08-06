const HEADER_TERMINATOR = Buffer.from('\r\n\r\n', 'ascii');
const MAX_HEADER_BYTES = 128;
const UTF8_DECODER = new TextDecoder('utf-8', { fatal: true });

export class FrameProtocolError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'FrameProtocolError';
  }
}

export interface FrameDecoderOptions {
  readonly maxFrameBytes?: number;
}

export function encodeFrame(message: unknown): Buffer {
  let serialized: string;
  try {
    serialized = JSON.stringify(message);
  } catch {
    throw new FrameProtocolError('Outbound protocol message is not JSON serializable.');
  }
  if (serialized === undefined) {
    throw new FrameProtocolError('Outbound protocol message is not a JSON value.');
  }
  const body = Buffer.from(serialized, 'utf8');
  const header = Buffer.from(
    `Content-Length: ${body.byteLength}\r\n\r\n`,
    'ascii',
  );
  return Buffer.concat([header, body]);
}

export class FrameDecoder {
  readonly #maxFrameBytes: number | undefined;
  #buffer = Buffer.alloc(0);

  constructor(options: FrameDecoderOptions = {}) {
    this.#maxFrameBytes = options.maxFrameBytes;
  }

  push(chunk: Uint8Array): unknown[] {
    if (chunk.byteLength > 0) {
      this.#buffer = Buffer.concat([this.#buffer, chunk]);
    }
    const messages: unknown[] = [];

    while (true) {
      const headerEnd = this.#buffer.indexOf(HEADER_TERMINATOR);
      if (headerEnd < 0) {
        if (this.#buffer.byteLength > MAX_HEADER_BYTES) {
          throw new FrameProtocolError('Protocol frame header exceeds 128 bytes.');
        }
        break;
      }
      if (headerEnd > MAX_HEADER_BYTES) {
        throw new FrameProtocolError('Protocol frame header exceeds 128 bytes.');
      }

      const header = this.#buffer.subarray(0, headerEnd).toString('ascii');
      const match = /^Content-Length: ([0-9]+)$/.exec(header);
      if (match === null) {
        throw new FrameProtocolError('Protocol frame has an invalid Content-Length header.');
      }
      const contentLength = Number(match[1]);
      if (!Number.isSafeInteger(contentLength)) {
        throw new FrameProtocolError('Protocol Content-Length is not a safe integer.');
      }
      if (
        this.#maxFrameBytes !== undefined &&
        contentLength > this.#maxFrameBytes
      ) {
        throw new FrameProtocolError(
          `Protocol frame declares ${contentLength} bytes, exceeding the configured limit.`,
        );
      }

      const bodyStart = headerEnd + HEADER_TERMINATOR.byteLength;
      const frameEnd = bodyStart + contentLength;
      if (this.#buffer.byteLength < frameEnd) break;

      try {
        const body = UTF8_DECODER.decode(
          this.#buffer.subarray(bodyStart, frameEnd),
        );
        messages.push(JSON.parse(body));
      } catch {
        throw new FrameProtocolError(
          'Protocol frame body is not valid UTF-8 JSON.',
        );
      }
      this.#buffer = this.#buffer.subarray(frameEnd);
    }

    return messages;
  }

  finish(): void {
    if (this.#buffer.byteLength !== 0) {
      throw new FrameProtocolError('Protocol stream ended with an incomplete frame.');
    }
  }
}

export type FrameWrite = (frame: Uint8Array) => void | Promise<void>;

export class SerializedFrameWriter {
  readonly #write: FrameWrite;
  #tail: Promise<void> = Promise.resolve();

  constructor(write: FrameWrite) {
    this.#write = write;
  }

  send(message: unknown): Promise<void> {
    const frame = encodeFrame(message);
    const pending = this.#tail.then(async () => {
      await this.#write(frame);
    });
    this.#tail = pending.catch(() => {});
    return pending;
  }

  async drain(): Promise<void> {
    await this.#tail;
  }
}
