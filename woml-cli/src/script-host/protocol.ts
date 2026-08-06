import Ajv2020, {
  type ErrorObject,
  type ValidateFunction,
} from 'ajv/dist/2020';

import attemptFailureV1Schema from '../../../docs/schemas/attempt-failure.v1.schema.json';
import attemptFailureV2Schema from '../../../docs/schemas/attempt-failure.v2.schema.json';
import scriptHostProtocolV1Schema from '../../../docs/schemas/script-host-protocol.v1.schema.json';
import scriptHostProtocolV2Schema from '../../../docs/schemas/script-host-protocol.v2.schema.json';
import type { CancelMessage, ExecuteMessage } from './types';

const ajv = new Ajv2020({ allErrors: true, strict: false });
ajv.addSchema(attemptFailureV1Schema);
ajv.addSchema(attemptFailureV2Schema);
const validateV1 = ajv.compile(scriptHostProtocolV1Schema) as ValidateFunction;
const validateV2 = ajv.compile(scriptHostProtocolV2Schema) as ValidateFunction;

export class MessageProtocolError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'MessageProtocolError';
  }
}

export function isScriptHostMessage(message: unknown): boolean {
  return validateV1(message) || validateV2(message);
}

function validatorFor(message: unknown): ValidateFunction {
  return typeof message === 'object' &&
    message !== null &&
    'protocolVersion' in message &&
    message.protocolVersion === 2
    ? validateV2
    : validateV1;
}

function describeErrors(errors: ErrorObject[] | null | undefined): string {
  if (errors === undefined || errors === null || errors.length === 0) {
    return 'unknown schema violation';
  }
  return errors
    .slice(0, 3)
    .map(
      error => `${error.instancePath || '/'} ${error.message ?? error.keyword}`
    )
    .join('; ');
}

export function assertExecuteMessage(
  message: unknown
): asserts message is ExecuteMessage {
  const validator = validatorFor(message);
  if (!validator(message)) {
    throw new MessageProtocolError(
      `Message does not match a supported script-host protocol: ${describeErrors(validator.errors)}.`
    );
  }
  if (
    typeof message !== 'object' ||
    message === null ||
    !('messageType' in message) ||
    message.messageType !== 'execute'
  ) {
    throw new MessageProtocolError(
      'The Bun script host accepts execute messages from Rust only.'
    );
  }
}

export function assertInboundMessage(
  message: unknown,
  protocolVersion: 1 | 2
): asserts message is ExecuteMessage | CancelMessage {
  const validator = protocolVersion === 2 ? validateV2 : validateV1;
  if (!validator(message)) {
    throw new MessageProtocolError(
      `Message does not match script-host protocol v${protocolVersion}: ${describeErrors(validator.errors)}.`
    );
  }
  if (
    typeof message !== 'object' ||
    message === null ||
    !('protocolVersion' in message) ||
    message.protocolVersion !== protocolVersion ||
    !('messageType' in message) ||
    (message.messageType !== 'execute' && message.messageType !== 'cancel')
  ) {
    throw new MessageProtocolError(
      `The Bun script host accepts protocol-v${protocolVersion} execute${protocolVersion === 2 ? ' and cancel' : ''} messages only.`
    );
  }
}
