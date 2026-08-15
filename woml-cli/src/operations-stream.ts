export interface OperationsStreamEventV1 {
  readonly profile: 'woml.runtime-operations-stream/v1';
  readonly runtimeInstanceId: string;
  readonly sequence: number;
  readonly occurredAt: string;
  readonly kind:
    | 'runtime'
    | 'workflow'
    | 'run'
    | 'trigger'
    | 'approval'
    | 'retry'
    | 'workflow_call'
    | 'policy'
    | 'provider'
    | 'storage'
    | 'maintenance'
    | 'alert';
  readonly subject: {
    readonly id: string;
    readonly status: string;
    readonly code?: string;
  };
}

function record(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function nonEmptyString(value: unknown, maximum = 2_048): value is string {
  return typeof value === 'string' && value.length > 0 && value.length <= maximum;
}

export function decodeOperationsStreamEvent(
  value: unknown
): OperationsStreamEventV1 {
  if (
    !record(value) ||
    value.profile !== 'woml.runtime-operations-stream/v1' ||
    !nonEmptyString(value.runtimeInstanceId, 320) ||
    !Number.isSafeInteger(value.sequence) ||
    Number(value.sequence) < 1 ||
    !nonEmptyString(value.occurredAt, 64) ||
    !Number.isFinite(Date.parse(value.occurredAt)) ||
    ![
      'runtime',
      'workflow',
      'run',
      'trigger',
      'approval',
      'retry',
      'workflow_call',
      'policy',
      'provider',
      'storage',
      'maintenance',
      'alert',
    ].includes(String(value.kind)) ||
    !record(value.subject) ||
    !nonEmptyString(value.subject.id, 320) ||
    !nonEmptyString(value.subject.status, 64) ||
    (value.subject.code !== undefined &&
      (typeof value.subject.code !== 'string' ||
        !/^WOML_[A-Z0-9_]+$/.test(value.subject.code)))
  ) {
    throw new Error('The runtime returned an invalid operations stream event.');
  }
  return value as unknown as OperationsStreamEventV1;
}

export async function consumeOperationsStream(
  response: Response,
  receive: (event: OperationsStreamEventV1) => void | Promise<void>,
  signal?: AbortSignal
): Promise<void> {
  if (!response.ok || response.body === null) {
    throw new Error(`Runtime event stream failed with HTTP ${response.status}.`);
  }
  const reader = response.body.getReader();
  const cancel = (): void => {
    void reader.cancel().catch(() => {});
  };
  signal?.addEventListener('abort', cancel, { once: true });
  const decoder = new TextDecoder();
  let buffered = '';
  try {
    for (;;) {
      const chunk = await reader.read();
      if (chunk.done) break;
      buffered += decoder.decode(chunk.value, { stream: true });
      for (;;) {
        const boundary = buffered.match(/\r?\n\r?\n/);
        if (boundary?.index === undefined) break;
        const block = buffered.slice(0, boundary.index);
        buffered = buffered.slice(boundary.index + boundary[0].length);
        const data = block
          .split(/\r?\n/)
          .find(line => line.startsWith('data:'));
        if (data !== undefined) {
          await receive(
            decodeOperationsStreamEvent(JSON.parse(data.slice(5).trimStart()))
          );
        }
      }
    }
  } finally {
    signal?.removeEventListener('abort', cancel);
  }
}
