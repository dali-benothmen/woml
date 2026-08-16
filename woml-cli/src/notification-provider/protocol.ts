import { isValidSecretName } from 'woml';

import {
  INFORMATIONAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
  NOTIFICATION_PROVIDER_PROTOCOL,
  NOTIFICATION_PROVIDER_PROTOCOL_VERSION,
  type NotificationInvocation,
  type NotificationProviderFailure,
  type ProviderMessageIdentity,
} from './types';

export class NotificationProtocolError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'NotificationProtocolError';
  }
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

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

const ID = /^.{1,320}$/s;
const APPROVAL_ID = /^[a-z][A-Za-z0-9]*$/;
const REQUEST_ID = /^aprreq_[A-Za-z0-9_-]+$/;
const DELIVERY_ID =
  /^[a-z][A-Za-z0-9]*:notify:(0|[1-9][0-9]*):(channel|chat):(0|[1-9][0-9]*)$/;
const UPDATE_ID = /^nupdate_[A-Za-z0-9_-]+$/;
const SHA256 = /^sha256:[a-f0-9]{64}$/;
const SLACK_DESTINATION = /^(#[a-z0-9][a-z0-9_-]{0,79}|[CG][A-Z0-9]{8,31})$/;
const TELEGRAM_DESTINATION = /^-?[1-9][0-9]{0,19}$/;

function secretReference(value: unknown): boolean {
  return (
    record(value) &&
    exactKeys(value, ['kind', 'name']) &&
    value.kind === 'secretReference' &&
    typeof value.name === 'string' &&
    value.name.length <= 255 &&
    isValidSecretName(value.name)
  );
}

function credentials(value: unknown, provider: unknown): boolean {
  if (!record(value)) return false;
  if (provider === 'telegram') {
    return exactKeys(value, ['botToken']) && secretReference(value.botToken);
  }
  return provider === 'slack' &&
    exactKeys(value, ['botToken', 'appToken']) &&
    secretReference(value.botToken) &&
    secretReference(value.appToken);
}

export function validProviderMessage(
  value: unknown
): value is ProviderMessageIdentity {
  if (
    record(value) &&
    exactKeys(value, ['provider', 'accountId', 'conversationId', 'messageId']) &&
    value.provider === 'telegram' &&
    typeof value.accountId === 'string' &&
    /^-?[1-9][0-9]{0,19}$/.test(value.accountId) &&
    typeof value.conversationId === 'string' &&
    TELEGRAM_DESTINATION.test(value.conversationId) &&
    typeof value.messageId === 'string' &&
    /^[1-9][0-9]{0,19}$/.test(value.messageId)
  ) return true;
  return (
    record(value) &&
    exactKeys(value, ['workspaceId', 'channelId', 'messageId']) &&
    typeof value.workspaceId === 'string' &&
    /^T[A-Z0-9]{8,31}$/.test(value.workspaceId) &&
    typeof value.channelId === 'string' &&
    /^[CGD][A-Z0-9]{8,31}$/.test(value.channelId) &&
    typeof value.messageId === 'string' &&
    /^[0-9]{10,}\.[0-9]{6}$/.test(value.messageId)
  );
}

function approvalBase(value: Record<string, unknown>): boolean {
  return (
    value.protocol === NOTIFICATION_PROVIDER_PROTOCOL &&
    value.protocolVersion === NOTIFICATION_PROVIDER_PROTOCOL_VERSION &&
    typeof value.invocationId === 'string' &&
    ID.test(value.invocationId) &&
    typeof value.runId === 'string' &&
    ID.test(value.runId) &&
    typeof value.approvalId === 'string' &&
    APPROVAL_ID.test(value.approvalId) &&
    typeof value.requestId === 'string' &&
    REQUEST_ID.test(value.requestId) &&
    typeof value.deliveryId === 'string' &&
    DELIVERY_ID.test(value.deliveryId) &&
    (value.provider === 'slack' || value.provider === 'telegram') &&
    credentials(value.credentials, value.provider)
  );
}

function informationalBase(value: Record<string, unknown>): boolean {
  return (
    value.protocol === NOTIFICATION_PROVIDER_PROTOCOL &&
    value.protocolVersion ===
      INFORMATIONAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION &&
    typeof value.invocationId === 'string' &&
    ID.test(value.invocationId) &&
    typeof value.runId === 'string' &&
    ID.test(value.runId) &&
    typeof value.hookInvocationId === 'string' &&
    SHA256.test(value.hookInvocationId) &&
    typeof value.actionId === 'string' &&
    ID.test(value.actionId) &&
    typeof value.deliveryId === 'string' &&
    ID.test(value.deliveryId) &&
    (value.provider === 'slack' || value.provider === 'telegram') &&
    typeof value.destination === 'string' &&
    (value.provider === 'slack'
      ? SLACK_DESTINATION.test(value.destination)
      : TELEGRAM_DESTINATION.test(value.destination)) &&
    typeof value.idempotencyKey === 'string' &&
    SHA256.test(value.idempotencyKey) &&
    credentials(value.credentials, value.provider)
  );
}

function approvalMessage(value: unknown): boolean {
  return (
    record(value) &&
    exactKeys(
      value,
      ['workflowId', 'approvalName'],
      ['approvalDescription', 'expiresAt']
    ) &&
    typeof value.workflowId === 'string' &&
    ID.test(value.workflowId) &&
    typeof value.approvalName === 'string' &&
    value.approvalName.length >= 1 &&
    value.approvalName.length <= 256 &&
    (value.approvalDescription === undefined ||
      (typeof value.approvalDescription === 'string' &&
        value.approvalDescription.length >= 1 &&
        value.approvalDescription.length <= 2000)) &&
    (value.expiresAt === undefined ||
      (typeof value.expiresAt === 'string' &&
        Number.isFinite(Date.parse(value.expiresAt))))
  );
}

export function assertNotificationInvocation(
  value: unknown
): asserts value is NotificationInvocation {
  if (!record(value)) {
    throw new NotificationProtocolError(
      'The provider host received an invalid invocation envelope.'
    );
  }
  if (value.protocolVersion === INFORMATIONAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION) {
    if (
      !informationalBase(value) ||
      !exactKeys(value, [
        'protocol',
        'protocolVersion',
        'messageType',
        'mode',
        'invocationId',
        'runId',
        'hookInvocationId',
        'actionId',
        'deliveryId',
        'provider',
        'destination',
        'idempotencyKey',
        'credentials',
        'message',
      ]) ||
      value.messageType !== 'deliver' ||
      value.mode !== 'informational' ||
      typeof value.message !== 'string' ||
      value.message.length < 1 ||
      [...value.message].length > 4096
    ) {
      throw new NotificationProtocolError(
        'The provider host received an invalid informational delivery invocation.'
      );
    }
    return;
  }
  if (!approvalBase(value)) {
    throw new NotificationProtocolError(
      'The provider host received an invalid invocation envelope.'
    );
  }
  if (value.messageType === 'deliver') {
    if (
      !exactKeys(
        value,
        [
          'protocol',
          'protocolVersion',
          'messageType',
          'invocationId',
          'runId',
          'approvalId',
          'requestId',
          'deliveryId',
          'provider',
          'destination',
          'idempotencyKey',
          'credentials',
          'decisionCapability',
          'message',
        ]
      ) ||
      typeof value.destination !== 'string' ||
      !(value.provider === 'slack'
        ? SLACK_DESTINATION.test(value.destination)
        : TELEGRAM_DESTINATION.test(value.destination)) ||
      typeof value.idempotencyKey !== 'string' ||
      !SHA256.test(value.idempotencyKey) ||
      typeof value.decisionCapability !== 'string' ||
      value.decisionCapability.length < 43 ||
      value.decisionCapability.length > 512 ||
      !approvalMessage(value.message)
    ) {
      throw new NotificationProtocolError(
        'The provider host received an invalid delivery invocation.'
      );
    }
    return;
  }
  if (
    value.messageType !== 'update' ||
    !exactKeys(value, [
      'protocol',
      'protocolVersion',
      'messageType',
      'invocationId',
      'runId',
      'approvalId',
      'requestId',
      'deliveryId',
      'updateId',
      'idempotencyKey',
      'provider',
      'credentials',
      'providerMessage',
      'resolution',
    ]) ||
    typeof value.updateId !== 'string' ||
    value.updateId.length > 256 ||
    !UPDATE_ID.test(value.updateId) ||
    typeof value.idempotencyKey !== 'string' ||
    !SHA256.test(value.idempotencyKey) ||
    !validProviderMessage(value.providerMessage) ||
    (value.resolution !== 'approved' &&
      value.resolution !== 'rejected' &&
      value.resolution !== 'timeout_failed')
  ) {
    throw new NotificationProtocolError(
      'The provider host received an invalid update invocation.'
    );
  }
}

const FAILURE_KINDS = new Set<NotificationProviderFailure['kind']>([
  'secret_not_found',
  'provider_auth_failed',
  'destination_invalid',
  'rate_limited',
  'provider_unavailable',
  'delivery_ambiguous',
  'request_invalid',
  'host_crashed',
  'size_limit_exceeded',
  'update_failed',
]);

export function validFailure(value: unknown): value is NotificationProviderFailure {
  return (
    record(value) &&
    exactKeys(value, ['kind', 'code', 'message', 'retryable'], [
      'retryAfterMs',
    ]) &&
    typeof value.kind === 'string' &&
    FAILURE_KINDS.has(value.kind as NotificationProviderFailure['kind']) &&
    typeof value.code === 'string' &&
    /^WOML_[A-Z0-9_]+$/.test(value.code) &&
    typeof value.message === 'string' &&
    value.message.length >= 1 &&
    value.message.length <= 1024 &&
    typeof value.retryable === 'boolean' &&
    (value.retryAfterMs === undefined ||
      (Number.isSafeInteger(value.retryAfterMs) &&
        Number(value.retryAfterMs) >= 0 &&
        Number(value.retryAfterMs) <= 86_400_000))
  );
}
