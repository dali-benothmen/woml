import Ajv2020, {
  type ErrorObject,
  type ValidateFunction,
} from 'ajv/dist/2020';

import attemptFailureV1Schema from '../../../docs/schemas/attempt-failure.v1.schema.json';
import attemptFailureV2Schema from '../../../docs/schemas/attempt-failure.v2.schema.json';
import attemptFailureV3Schema from '../../../docs/schemas/attempt-failure.v3.schema.json';
import capabilityCallV1Schema from '../../../docs/schemas/capability-call.v1.schema.json';
import nativeFetchObservationV1Schema from '../../../docs/schemas/native-fetch-observation.v1.schema.json';
import scriptHostProtocolV1Schema from '../../../docs/schemas/script-host-protocol.v1.schema.json';
import scriptHostProtocolV2Schema from '../../../docs/schemas/script-host-protocol.v2.schema.json';
import scriptHostProtocolV3Schema from '../../../docs/schemas/script-host-protocol.v3.schema.json';
import scriptHostProtocolV4Schema from '../../../docs/schemas/script-host-protocol.v4.schema.json';
import type {
  CancelMessage,
  ExecuteMessage,
  ScriptHostProtocolVersion,
} from './types';

const ajv = new Ajv2020({ allErrors: true, strict: false, logger: false });
ajv.addSchema(attemptFailureV1Schema);
ajv.addSchema(attemptFailureV2Schema);
ajv.addSchema(attemptFailureV3Schema);
ajv.addSchema(capabilityCallV1Schema);
ajv.addSchema(nativeFetchObservationV1Schema);
const validateV1 = ajv.compile(scriptHostProtocolV1Schema) as ValidateFunction;
const validateV2 = ajv.compile(scriptHostProtocolV2Schema) as ValidateFunction;
const validateV3 = ajv.compile(scriptHostProtocolV3Schema) as ValidateFunction;
const validateV4 = ajv.compile(scriptHostProtocolV4Schema) as ValidateFunction;

export class MessageProtocolError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'MessageProtocolError';
  }
}

export function isScriptHostMessage(message: unknown): boolean {
  return (
    validateV1(message) ||
    validateV2(message) ||
    validateV3(message) ||
    validateV4(message)
  );
}

function validatorFor(message: unknown): ValidateFunction {
  if (typeof message !== 'object' || message === null) return validateV1;
  if (!('protocolVersion' in message)) return validateV1;
  if (message.protocolVersion === 4) return validateV4;
  if (message.protocolVersion === 3) return validateV3;
  if (message.protocolVersion === 2) return validateV2;
  return validateV1;
}

function validateAttemptSemantics(message: ExecuteMessage): void {
  if (
    (message.protocolVersion === 3 || message.protocolVersion === 4) &&
    message.attempt.number > message.attempt.maxAttempts
  ) {
    throw new MessageProtocolError(
      `Script-host protocol v${message.protocolVersion} attempt.number must not exceed attempt.maxAttempts.`
    );
  }
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
  validateAttemptSemantics(message as ExecuteMessage);
}

export function assertInboundMessage(
  message: unknown,
  protocolVersion: ScriptHostProtocolVersion
): asserts message is
  | ExecuteMessage
  | CancelMessage
  | import('./types').CapabilityResultMessage
  | import('./types').FetchObservationAckMessage {
  const validator =
    protocolVersion === 4
      ? validateV4
      : protocolVersion === 3
        ? validateV3
        : protocolVersion === 2
          ? validateV2
          : validateV1;
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
    (message.messageType !== 'execute' &&
      message.messageType !== 'cancel' &&
      message.messageType !== 'capability_result' &&
      message.messageType !== 'fetch_observation_ack')
  ) {
    throw new MessageProtocolError(
      `The Bun script host received an unsupported protocol-v${protocolVersion} message direction.`
    );
  }
  if (message.messageType === 'execute') {
    validateAttemptSemantics(message as ExecuteMessage);
  }
}
