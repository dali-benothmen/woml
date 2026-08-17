import Ajv2020, {
  type ErrorObject,
  type ValidateFunction,
} from 'ajv/dist/2020';
import addFormats from 'ajv-formats';

import customProviderProtocolV1Schema from '../../../docs/schemas/custom-notification-provider.v1.schema.json';
import type {
  CustomProviderCompletedMessage,
  CustomProviderExecuteMessage,
  CustomProviderInbound,
  CustomProviderOutbound,
  CustomProviderReadyMessage,
} from './types';

const ajv = new Ajv2020({ allErrors: true, strict: false, logger: false });
addFormats(ajv);
const validate = ajv.compile(
  customProviderProtocolV1Schema
) as ValidateFunction;

export class CustomProviderProtocolError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'CustomProviderProtocolError';
  }
}

function describeErrors(
  errors: ErrorObject[] | null | undefined
): string {
  if (errors === undefined || errors === null || errors.length === 0) {
    return 'unknown schema violation';
  }
  return errors
    .slice(0, 3)
    .map(error => `${error.instancePath || '/'} ${error.message ?? error.keyword}`)
    .join('; ');
}

function assertSchema(message: unknown): void {
  if (!validate(message)) {
    throw new CustomProviderProtocolError(
      `Message does not match Custom Notification Provider Protocol v1: ${describeErrors(validate.errors)}.`
    );
  }
}

function messageType(message: unknown): unknown {
  return typeof message === 'object' && message !== null
    ? (message as { messageType?: unknown }).messageType
    : undefined;
}

export function assertCustomProviderInbound(
  message: unknown,
  knownScriptArtifactIds?: ReadonlySet<string>
): asserts message is CustomProviderInbound {
  assertSchema(message);
  const type = messageType(message);
  if (type !== 'execute' && type !== 'cancel') {
    throw new CustomProviderProtocolError(
      'The Bun custom-provider host accepts execute and cancel messages from Rust only.'
    );
  }
  if (type === 'execute') {
    const execute = message as CustomProviderExecuteMessage;
    if (execute.attempt.number > execute.attempt.max) {
      throw new CustomProviderProtocolError(
        'Custom-provider attempt.number must not exceed attempt.max.'
      );
    }
    if (
      knownScriptArtifactIds !== undefined &&
      !knownScriptArtifactIds.has(execute.scriptArtifactId)
    ) {
      throw new CustomProviderProtocolError(
        `Custom-provider script artifact "${execute.scriptArtifactId}" is not registered.`
      );
    }
  }
}

export function assertCustomProviderOutbound(
  message: unknown
): asserts message is CustomProviderOutbound {
  assertSchema(message);
  const type = messageType(message);
  if (type !== 'ready' && type !== 'completed') {
    throw new CustomProviderProtocolError(
      'The Bun custom-provider host may emit ready and completed messages only.'
    );
  }
}

export function assertCustomProviderReady(
  message: unknown
): asserts message is CustomProviderReadyMessage {
  assertCustomProviderOutbound(message);
  if (message.messageType !== 'ready') {
    throw new CustomProviderProtocolError(
      'The custom-provider host did not send its ready message.'
    );
  }
}

export function assertCustomProviderCompleted(
  message: unknown
): asserts message is CustomProviderCompletedMessage {
  assertCustomProviderOutbound(message);
  if (message.messageType !== 'completed') {
    throw new CustomProviderProtocolError(
      'The custom-provider host did not send a completion message.'
    );
  }
}
