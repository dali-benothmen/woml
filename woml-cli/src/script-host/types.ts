export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonObject | JsonValue[];
export interface JsonObject {
  readonly [key: string]: JsonValue;
}

export interface ScriptContext extends JsonObject {
  readonly trigger: JsonObject;
  readonly steps: Readonly<Record<string, JsonValue>>;
}

export type ScriptHostProtocolVersion = 1 | 2 | 3;

export interface ScriptAttempt extends JsonObject {
  readonly number: number;
  readonly maxAttempts: number;
  readonly idempotencyKey: string;
}

export interface ReadyMessage {
  readonly protocol: 'woml.script-host';
  readonly protocolVersion: ScriptHostProtocolVersion;
  readonly messageType: 'ready';
  readonly hostInstanceId: string;
}

interface ExecuteMessageBase {
  readonly protocol: 'woml.script-host';
  readonly messageType: 'execute';
  readonly invocationId: string;
  readonly runId: string;
  readonly nodeId: string;
  readonly handler: 'runtime.script';
  readonly timeoutMs: number;
  readonly source: string;
  readonly context: ScriptContext;
}

export interface LegacyExecuteMessage extends ExecuteMessageBase {
  readonly protocolVersion: 1 | 2;
  readonly attempt: number;
}

export interface ExecuteMessageV3 extends ExecuteMessageBase {
  readonly protocolVersion: 3;
  readonly attempt: ScriptAttempt;
}

export type ExecuteMessage = LegacyExecuteMessage | ExecuteMessageV3;

export interface CancelMessage {
  readonly protocol: 'woml.script-host';
  readonly protocolVersion: 2 | 3;
  readonly messageType: 'cancel';
  readonly invocationId: string;
  readonly reason: 'parallel_fail_fast';
}

export type HostReportedFailureKind =
  | 'script_threw'
  | 'script_timed_out'
  | 'invalid_script_result'
  | 'context_too_large'
  | 'result_too_large'
  | 'worker_crashed'
  | 'invocation_cancelled';

export type HostReportedFailureCode =
  | 'WOML_SCRIPT_THROWN'
  | 'WOML_SCRIPT_TIMEOUT'
  | 'WOML_SCRIPT_NON_JSON_RESULT'
  | 'WOML_SCRIPT_CONTEXT_TOO_LARGE'
  | 'WOML_SCRIPT_RESULT_TOO_LARGE'
  | 'WOML_SCRIPT_WORKER_CRASHED'
  | 'WOML_SCRIPT_CANCELLED';

export interface HostReportedFailure {
  readonly kind: HostReportedFailureKind;
  readonly code: HostReportedFailureCode;
  readonly message: string;
  readonly details?: {
    readonly actualBytes?: number;
    readonly limitBytes?: number;
  };
}

export interface SuccessMessage {
  readonly protocol: 'woml.script-host';
  readonly protocolVersion: ScriptHostProtocolVersion;
  readonly messageType: 'completed';
  readonly invocationId: string;
  readonly outcome: {
    readonly kind: 'success';
    readonly value: JsonValue;
  };
  readonly durationMs: number;
}

export interface FailureMessage {
  readonly protocol: 'woml.script-host';
  readonly protocolVersion: ScriptHostProtocolVersion;
  readonly messageType: 'completed';
  readonly invocationId: string;
  readonly outcome: {
    readonly kind: 'failure';
    readonly error: HostReportedFailure;
  };
  readonly durationMs: number;
}

export type CompletedMessage = SuccessMessage | FailureMessage;
export type ScriptHostMessage =
  | ReadyMessage
  | ExecuteMessage
  | CancelMessage
  | CompletedMessage;

export interface ScriptHostLimits {
  readonly maxContextBytes?: number;
  readonly maxResultBytes?: number;
  readonly maxFrameBytes?: number;
}
