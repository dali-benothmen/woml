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
import scriptHostProtocolV5Schema from '../../../docs/schemas/script-host-protocol.v5.schema.json';
import scriptHostProtocolV6Schema from '../../../docs/schemas/script-host-protocol.v6.schema.json';
import scriptHostProtocolV7Schema from '../../../docs/schemas/script-host-protocol.v7.schema.json';
import scriptHostProtocolV8Schema from '../../../docs/schemas/script-host-protocol.v8.schema.json';
import lifecycleBindingV1Schema from '../../../docs/schemas/lifecycle-binding.v1.schema.json';
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
ajv.addSchema(lifecycleBindingV1Schema);
const validateV1 = ajv.compile(scriptHostProtocolV1Schema) as ValidateFunction;
const validateV2 = ajv.compile(scriptHostProtocolV2Schema) as ValidateFunction;
const validateV3 = ajv.compile(scriptHostProtocolV3Schema) as ValidateFunction;
const validateV4 = ajv.compile(scriptHostProtocolV4Schema) as ValidateFunction;
const validateV5 = ajv.compile(scriptHostProtocolV5Schema) as ValidateFunction;
const validateV6 = ajv.compile(scriptHostProtocolV6Schema) as ValidateFunction;
const validateV7 = ajv.compile(scriptHostProtocolV7Schema) as ValidateFunction;
const validateV8Schema = ajv.compile(scriptHostProtocolV8Schema) as ValidateFunction;

function exactKeys(
  value: Record<string, unknown>,
  required: readonly string[],
  optional: readonly string[] = []
): boolean {
  const allowed = new Set([...required, ...optional]);
  return (
    required.every(key => Object.hasOwn(value, key)) &&
    Object.keys(value).every(key => allowed.has(key))
  );
}

function validReusableBinding(value: unknown): boolean {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    return false;
  }
  const binding = value as Record<string, unknown>;
  const definition = binding.definition;
  const props = binding.props;
  return (
    exactKeys(binding, ['profile', 'invocationId', 'definition', 'props']) &&
    binding.profile === 'woml.reusable-script-binding/v3' &&
    typeof binding.invocationId === 'string' &&
    binding.invocationId.length >= 1 &&
    binding.invocationId.length <= 256 &&
    typeof definition === 'object' &&
    definition !== null &&
    !Array.isArray(definition) &&
    exactKeys(definition as Record<string, unknown>, [
      'kind',
      'alias',
      'digest',
      'source',
    ]) &&
    ((definition as Record<string, unknown>).kind === 'step' ||
      (definition as Record<string, unknown>).kind ===
        'notification-provider') &&
    typeof (definition as Record<string, unknown>).alias === 'string' &&
    /^sha256:[0-9a-f]{64}$/.test(
      String((definition as Record<string, unknown>).digest)
    ) &&
    typeof (definition as Record<string, unknown>).source === 'string' &&
    typeof props === 'object' &&
    props !== null &&
    !Array.isArray(props) &&
    Object.keys(props).length <= 64 &&
    Buffer.byteLength(JSON.stringify(props), 'utf8') <= 1_048_576
  );
}

function validReusableLifecycle(value: unknown): boolean {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    return false;
  }
  const lifecycle = value as Record<string, unknown>;
  return (
    exactKeys(lifecycle, ['hook', 'outcome'], ['result', 'error']) &&
    (lifecycle.hook === 'on-success' ||
      lifecycle.hook === 'on-error' ||
      lifecycle.hook === 'on-complete') &&
    (lifecycle.outcome === 'succeeded' ||
      lifecycle.outcome === 'failed' ||
      lifecycle.outcome === 'cancelled') &&
    (lifecycle.error === undefined ||
      (typeof lifecycle.error === 'object' &&
        lifecycle.error !== null &&
        !Array.isArray(lifecycle.error) &&
        exactKeys(lifecycle.error as Record<string, unknown>, [
          'code',
          'message',
        ]) &&
        typeof (lifecycle.error as Record<string, unknown>).code ===
          'string' &&
        typeof (lifecycle.error as Record<string, unknown>).message ===
          'string'))
  );
}

function validExecuteV8Base(record: Record<string, unknown>): boolean {
  const attempt = record.attempt as Record<string, unknown> | undefined;
  const context = record.context as Record<string, unknown> | undefined;
  const bindings = record.bindings as Record<string, unknown> | undefined;
  const modules = record.modules;
  return (
    exactKeys(
      record,
      [
        'protocol', 'protocolVersion', 'messageType', 'invocationId', 'runId',
        'nodeId', 'attempt', 'mode', 'handler', 'timeoutMs', 'source', 'context',
        'bindings', 'modules',
      ],
      ['lifecycle', 'reusable', 'reusableLifecycle']
    ) &&
    record.protocol === 'woml.script-host' &&
    record.protocolVersion === 8 &&
    record.messageType === 'execute' &&
    [record.invocationId, record.runId, record.nodeId].every(
      value => typeof value === 'string' && value.length > 0 && value.length <= 256
    ) &&
    typeof attempt === 'object' && attempt !== null && !Array.isArray(attempt) &&
    exactKeys(attempt, ['number', 'maxAttempts', 'idempotencyKey']) &&
    Number.isInteger(attempt.number) && Number(attempt.number) >= 1 &&
    Number.isInteger(attempt.maxAttempts) && Number(attempt.maxAttempts) >= 1 &&
    /^sha256:[0-9a-f]{64}$/.test(String(attempt.idempotencyKey)) &&
    ((record.mode === 'step' && record.handler === 'runtime.script') ||
      (record.mode === 'lifecycle' && record.handler === 'runtime.lifecycle-script')) &&
    Number.isInteger(record.timeoutMs) && Number(record.timeoutMs) >= 1 &&
    Number(record.timeoutMs) <= 86_400_000 &&
    typeof record.source === 'string' &&
    typeof context === 'object' && context !== null && !Array.isArray(context) &&
    exactKeys(context, ['trigger', 'steps']) &&
    typeof context.trigger === 'object' && context.trigger !== null &&
    typeof context.steps === 'object' && context.steps !== null &&
    typeof bindings === 'object' && bindings !== null && !Array.isArray(bindings) &&
    exactKeys(bindings, ['bindingVersion', 'servicesVersion', 'secrets']) &&
    bindings.bindingVersion === 1 && bindings.servicesVersion === 1 &&
    typeof bindings.secrets === 'object' && bindings.secrets !== null &&
    Array.isArray(modules) && modules.length <= 64 &&
    modules.every(module => {
      if (typeof module !== 'object' || module === null || Array.isArray(module)) return false;
      const value = module as Record<string, unknown>;
      return exactKeys(value, ['name', 'bundleDigest', 'exports']) &&
        typeof value.name === 'string' && /^[a-z][A-Za-z0-9]*$/.test(value.name) &&
        /^sha256:[0-9a-f]{64}$/.test(String(value.bundleDigest)) &&
        Array.isArray(value.exports) && value.exports.length > 0 &&
        value.exports.every(item => typeof item === 'string');
    })
  );
}

function validateV8(message: unknown): boolean {
  if (typeof message !== 'object' || message === null || Array.isArray(message)) {
    return false;
  }
  const record = message as Record<string, unknown>;
  if (record.protocolVersion !== 8) return false;
  if (!validateV8Schema(message)) return false;
  if (record.messageType === 'execute') {
    if (!validExecuteV8Base(record)) return false;
    if (record.reusable === undefined) {
      return record.reusableLifecycle === undefined;
    }
    if (!validReusableBinding(record.reusable)) return false;
    if (record.reusableLifecycle === undefined) return record.mode === 'step';
    return record.mode === 'lifecycle' && validReusableLifecycle(record.reusableLifecycle);
  }
  const normalized: Record<string, unknown> = { ...record, protocolVersion: 7 };
  delete normalized.reusable;
  delete normalized.reusableLifecycle;
  if (!validateV7(normalized)) return false;
  return record.reusable === undefined && record.reusableLifecycle === undefined;
}

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
    validateV4(message) ||
    validateV5(message) ||
    validateV6(message) ||
    validateV7(message) ||
    validateV8(message)
  );
}

function validatorFor(message: unknown): ValidateFunction {
  if (typeof message !== 'object' || message === null) return validateV1;
  if (!('protocolVersion' in message)) return validateV1;
  if (message.protocolVersion === 8) return validateV7;
  if (message.protocolVersion === 7) return validateV7;
  if (message.protocolVersion === 6) return validateV6;
  if (message.protocolVersion === 5) return validateV5;
  if (message.protocolVersion === 4) return validateV4;
  if (message.protocolVersion === 3) return validateV3;
  if (message.protocolVersion === 2) return validateV2;
  return validateV1;
}

function validateAttemptSemantics(message: ExecuteMessage): void {
  if (
    (message.protocolVersion === 3 ||
      message.protocolVersion === 4 ||
      message.protocolVersion === 5 ||
      message.protocolVersion === 6 ||
      message.protocolVersion === 7 ||
      message.protocolVersion === 8) &&
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
    .slice(0, 5)
    .map(
      error => `${error.instancePath || '/'} ${error.message ?? error.keyword}`
    )
    .join('; ');
}

export function assertExecuteMessage(
  message: unknown
): asserts message is ExecuteMessage {
  const validator = validatorFor(message);
  const version8 =
    typeof message === 'object' &&
    message !== null &&
    'protocolVersion' in message &&
    message.protocolVersion === 8;
  if (!(version8 ? validateV8(message) : validator(message))) {
    throw new MessageProtocolError(
      `Message does not match a supported script-host protocol: ${describeErrors(version8 ? validateV8Schema.errors : validator.errors)}.`
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
  | import('./types').RegisterModuleMessage
  | import('./types').CapabilityResultMessage
  | import('./types').FetchObservationAckMessage {
  const validator =
    protocolVersion === 8
      ? validateV7
      : protocolVersion === 7
      ? validateV7
      : protocolVersion === 6
        ? validateV6
        : protocolVersion === 5
          ? validateV5
          : protocolVersion === 4
            ? validateV4
            : protocolVersion === 3
              ? validateV3
              : protocolVersion === 2
                ? validateV2
                : validateV1;
  if (!(protocolVersion === 8 ? validateV8(message) : validator(message))) {
    throw new MessageProtocolError(
      `Message does not match script-host protocol v${protocolVersion}: ${describeErrors(protocolVersion === 8 ? validateV8Schema.errors : validator.errors)}.`
    );
  }
  if (
    typeof message !== 'object' ||
    message === null ||
    !('protocolVersion' in message) ||
    message.protocolVersion !== protocolVersion ||
    !('messageType' in message) ||
    (message.messageType !== 'execute' &&
      message.messageType !== 'register_module' &&
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
