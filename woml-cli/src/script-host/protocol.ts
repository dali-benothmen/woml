import Ajv2020, { type ErrorObject, type ValidateFunction } from 'ajv/dist/2020';

import attemptFailureSchema from '../../../docs/schemas/attempt-failure.v1.schema.json';
import scriptHostProtocolSchema from '../../../docs/schemas/script-host-protocol.v1.schema.json';
import type { ExecuteMessage } from './types';

const ajv = new Ajv2020({ allErrors: true, strict: false });
ajv.addSchema(attemptFailureSchema);
const validateMessage = ajv.compile(
  scriptHostProtocolSchema,
) as ValidateFunction;

export class MessageProtocolError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'MessageProtocolError';
  }
}

export function isScriptHostMessage(message: unknown): boolean {
  return validateMessage(message);
}

function describeErrors(errors: ErrorObject[] | null | undefined): string {
  if (errors === undefined || errors === null || errors.length === 0) {
    return 'unknown schema violation';
  }
  return errors
    .slice(0, 3)
    .map(
      (error) =>
        `${error.instancePath || '/'} ${error.message ?? error.keyword}`,
    )
    .join('; ');
}

export function assertExecuteMessage(message: unknown): asserts message is ExecuteMessage {
  if (!isScriptHostMessage(message)) {
    throw new MessageProtocolError(
      `Message does not match script-host protocol v1: ${describeErrors(validateMessage.errors)}.`,
    );
  }
  if (
    typeof message !== 'object' ||
    message === null ||
    !('messageType' in message) ||
    message.messageType !== 'execute'
  ) {
    throw new MessageProtocolError(
      'The Bun script host accepts execute messages from Rust only.',
    );
  }
}
