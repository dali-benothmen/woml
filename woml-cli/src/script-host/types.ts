export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonObject | JsonValue[];
export interface JsonObject {
  readonly [key: string]: JsonValue;
}

export interface ScriptContext {
  readonly trigger: JsonObject;
  readonly steps: Readonly<Record<string, JsonValue>>;
  readonly item?: JsonValue;
  readonly iteration?: {
    readonly index: number;
    readonly total: number;
  };
}

export type ScriptHostProtocolVersion = 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9;

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
  readonly handler: 'runtime.script' | 'runtime.lifecycle-script';
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

export interface ScriptBindingsV1 {
  readonly bindingVersion: 1;
  readonly servicesVersion: 1;
  readonly secrets: Readonly<Record<string, string>>;
}

export interface ExecuteMessageV4 extends ExecuteMessageBase {
  readonly protocolVersion: 4;
  readonly attempt: ScriptAttempt;
  readonly bindings: ScriptBindingsV1;
}

export interface RuntimeModuleBindingV1 {
  readonly name: string;
  readonly bundleDigest: string;
  readonly exports: readonly string[];
}

export interface ExecuteMessageV5 extends ExecuteMessageBase {
  readonly protocolVersion: 5;
  readonly attempt: ScriptAttempt;
  readonly bindings: ScriptBindingsV1;
  readonly modules: readonly RuntimeModuleBindingV1[];
}

export interface ExecuteMessageV6 extends ExecuteMessageBase {
  readonly protocolVersion: 6;
  readonly attempt: ScriptAttempt;
  readonly bindings: ScriptBindingsV1;
  readonly modules: readonly RuntimeModuleBindingV1[];
}

export interface LifecycleBindingV1 {
  readonly event:
    | 'run_start'
    | 'step_start'
    | 'step_success'
    | 'step_failure'
    | 'step_complete'
    | 'run_success'
    | 'run_failure'
    | 'run_cancel'
    | 'run_complete';
  readonly workflow: {
    readonly id: string;
    readonly outcome?: 'succeeded' | 'failed' | 'cancelled';
  };
  readonly step?: {
    readonly id: string;
    readonly outcome?: 'succeeded' | 'failed' | 'cancelled';
    readonly attempts: number;
  };
  readonly failure?: { readonly code: string; readonly message: string };
}

export type ExecuteMessageV7 = ExecuteMessageBase & {
  readonly protocolVersion: 7;
  readonly attempt: ScriptAttempt;
  readonly bindings: ScriptBindingsV1;
  readonly modules: readonly RuntimeModuleBindingV1[];
} & (
    | { readonly mode: 'step'; readonly handler: 'runtime.script' }
    | {
        readonly mode: 'lifecycle';
        readonly handler: 'runtime.lifecycle-script';
        readonly lifecycle: LifecycleBindingV1;
      }
  );

export interface ReusableScriptBindingV3 {
  readonly profile: 'woml.reusable-script-binding/v3';
  readonly invocationId: string;
  readonly definition: {
    readonly kind: 'step' | 'notification-provider';
    readonly alias: string;
    readonly digest: string;
    readonly source: string;
  };
  readonly props: Readonly<Record<string, JsonValue>>;
}

export interface ReusableLifecycleBindingV1 {
  readonly hook: 'on-success' | 'on-error' | 'on-complete';
  readonly outcome: 'succeeded' | 'failed' | 'cancelled';
  readonly result?: JsonValue;
  readonly error?: { readonly code: string; readonly message: string };
}

export type ExecuteMessageV8 = ExecuteMessageBase & {
  readonly protocolVersion: 8;
  readonly attempt: ScriptAttempt;
  readonly bindings: ScriptBindingsV1;
  readonly modules: readonly RuntimeModuleBindingV1[];
  readonly mode: 'step' | 'lifecycle';
  readonly lifecycle?: LifecycleBindingV1;
  readonly reusable?: ReusableScriptBindingV3;
  readonly reusableLifecycle?: ReusableLifecycleBindingV1;
};

export type ExecuteMessageV9 = Omit<ExecuteMessageV8, 'protocolVersion'> & {
  readonly protocolVersion: 9;
};

export type ExecuteMessage =
  | LegacyExecuteMessage
  | ExecuteMessageV3
  | ExecuteMessageV4
  | ExecuteMessageV5
  | ExecuteMessageV6
  | ExecuteMessageV7
  | ExecuteMessageV8
  | ExecuteMessageV9;

export interface RegisterModuleMessageV5 {
  readonly protocol: 'woml.script-host';
  readonly protocolVersion: 5;
  readonly messageType: 'register_module';
  readonly bundleDigest: string;
  readonly bundle: string;
}

export interface RegisterModuleMessageV6 {
  readonly protocol: 'woml.script-host';
  readonly protocolVersion: 6 | 7 | 8 | 9;
  readonly messageType: 'register_module';
  readonly bundleDigest: string;
  readonly bundle: string;
  readonly sourceMapDigest: string;
  readonly sourceMap: string;
}

export type RegisterModuleMessage =
  | RegisterModuleMessageV5
  | RegisterModuleMessageV6;

type ModuleRegisteredMessageBase = {
  readonly protocol: 'woml.script-host';
  readonly messageType: 'module_registered';
  readonly bundleDigest: string;
};

export type ModuleRegisteredMessage = ModuleRegisteredMessageBase &
  (
    | { readonly protocolVersion: 5 }
    | { readonly protocolVersion: 6 | 7 | 8 | 9; readonly sourceMapDigest: string }
  ) &
  (
    | { readonly accepted: true }
    | {
        readonly accepted: false;
        readonly code:
          | 'WOML_MODULE_DIGEST_MISMATCH'
          | 'WOML_MODULE_CACHE_LIMIT_EXCEEDED';
        readonly message: string;
      }
  );

export interface CancelMessage {
  readonly protocol: 'woml.script-host';
  readonly protocolVersion: 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9;
  readonly messageType: 'cancel';
  readonly invocationId: string;
  readonly reason:
    | 'parallel_fail_fast'
    | 'step_timed_out'
    | 'run_cancelled'
    | 'host_shutdown';
}

export type CapabilityFailureKind =
  | 'invalid_input'
  | 'invalid_result'
  | 'unsupported_capability'
  | 'unsupported_operation'
  | 'unsupported_version'
  | 'input_too_large'
  | 'result_too_large'
  | 'frame_too_large'
  | 'timed_out'
  | 'cancelled'
  | 'handler_crashed'
  | 'worker_crashed'
  | 'host_crashed'
  | 'transport_failed'
  | 'service_rejected'
  | 'interrupted'
  | 'ambiguous';

export interface CapabilityFailure {
  readonly kind: CapabilityFailureKind;
  readonly code: string;
  readonly message: string;
  readonly retryable: boolean;
  readonly ambiguous: boolean;
  readonly details?: Readonly<Record<string, JsonPrimitive>>;
}

export interface CapabilityCallRequest {
  readonly contract: 'woml.capability-call';
  readonly contractVersion: 1;
  readonly messageType: 'request';
  readonly invocationId: string;
  readonly callId: string;
  readonly runId: string;
  readonly nodeId: string;
  readonly attemptNumber: number;
  readonly capability: string;
  readonly operation: string;
  readonly inputContractVersion: 1;
  readonly resultContractVersion: 1;
  readonly identity: {
    readonly mode: 'automatic' | 'named';
    readonly stepIdempotencyKey: string;
    readonly operationName: string;
    readonly operationKey: string;
    readonly providerIdempotencyKey?: string;
  };
  readonly limits: {
    readonly inputBytes: number;
    readonly resultBytes: number;
    readonly timeoutMs: number;
  };
  readonly input: JsonValue;
}

interface CapabilityResultBase {
  readonly contract: 'woml.capability-call';
  readonly contractVersion: 1;
  readonly messageType: 'result';
  readonly invocationId: string;
  readonly callId: string;
  readonly durationMs: number;
}

export type CapabilityCallResult =
  | (CapabilityResultBase & {
      readonly outcome: 'succeeded';
      readonly resultContractVersion: 1;
      readonly resultBytes: number;
      readonly result: JsonValue;
    })
  | (CapabilityResultBase & {
      readonly outcome: 'failed' | 'cancelled';
      readonly error: CapabilityFailure;
    });

export interface CapabilityCallMessage {
  readonly protocol: 'woml.script-host';
  readonly protocolVersion: 4 | 5 | 6 | 7 | 8 | 9;
  readonly messageType: 'capability_call';
  readonly invocationId: string;
  readonly callId: string;
  readonly call: CapabilityCallRequest;
}

export interface CapabilityResultMessage {
  readonly protocol: 'woml.script-host';
  readonly protocolVersion: 4 | 5 | 6 | 7 | 8 | 9;
  readonly messageType: 'capability_result';
  readonly invocationId: string;
  readonly callId: string;
  readonly result: CapabilityCallResult;
}

interface NativeFetchObservationBase {
  readonly contract: 'woml.native-fetch-observation';
  readonly contractVersion: 1;
  readonly invocationId: string;
  readonly requestId: string;
}

export type NativeFetchObservation =
  | (NativeFetchObservationBase & {
      readonly observationType: 'started';
      readonly method: string;
      readonly origin: string;
      readonly path: string;
      readonly requestBodyBytes?: number;
      readonly startedAt: string;
    })
  | (NativeFetchObservationBase & {
      readonly observationType: 'completed';
      readonly status: number;
      readonly responseBodyBytes: number | null;
      readonly durationMs: number;
      readonly completedAt: string;
    })
  | (NativeFetchObservationBase & {
      readonly observationType: 'failed';
      readonly durationMs: number;
      readonly failedAt: string;
      readonly error: {
        readonly kind:
          | 'tracking_failed'
          | 'fetch_rejected'
          | 'timed_out'
          | 'cancelled'
          | 'worker_crashed'
          | 'host_crashed';
        readonly code: string;
        readonly message: string;
      };
    });

export interface FetchObservationMessage {
  readonly protocol: 'woml.script-host';
  readonly protocolVersion: 4 | 5 | 6 | 7 | 8 | 9;
  readonly messageType: 'fetch_observation';
  readonly invocationId: string;
  readonly requestId: string;
  readonly observation: NativeFetchObservation;
}

export type FetchObservationAckMessage = {
  readonly protocol: 'woml.script-host';
  readonly protocolVersion: 4 | 5 | 6 | 7 | 8 | 9;
  readonly messageType: 'fetch_observation_ack';
  readonly invocationId: string;
  readonly requestId: string;
} & (
  | { readonly accepted: true }
  | { readonly accepted: false; readonly error: CapabilityFailure }
);

export type HostReportedFailureKind =
  | 'script_threw'
  | 'script_timed_out'
  | 'invalid_script_result'
  | 'context_too_large'
  | 'result_too_large'
  | 'worker_crashed'
  | 'invocation_cancelled'
  | 'service_failed';

export type HostReportedFailureCode =
  | 'WOML_SCRIPT_THROWN'
  | 'WOML_SCRIPT_TIMEOUT'
  | 'WOML_SCRIPT_NON_JSON_RESULT'
  | 'WOML_SCRIPT_CONTEXT_TOO_LARGE'
  | 'WOML_SCRIPT_RESULT_TOO_LARGE'
  | 'WOML_SCRIPT_WORKER_CRASHED'
  | 'WOML_SCRIPT_CANCELLED'
  | string;

export interface HostReportedFailure {
  readonly kind: HostReportedFailureKind;
  readonly code: HostReportedFailureCode;
  readonly message: string;
  readonly details?: {
    readonly actualBytes?: number;
    readonly limitBytes?: number;
  };
  readonly capability?: string;
  readonly operation?: string;
  readonly callId?: string;
  readonly retryable?: boolean;
  readonly ambiguous?: boolean;
  readonly cause?: CapabilityFailure;
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
  | RegisterModuleMessage
  | ModuleRegisteredMessage
  | ExecuteMessage
  | CancelMessage
  | CompletedMessage
  | CapabilityCallMessage
  | CapabilityResultMessage
  | FetchObservationMessage
  | FetchObservationAckMessage;

export interface ScriptHostLimits {
  readonly maxContextBytes?: number;
  readonly maxResultBytes?: number;
  readonly maxFrameBytes?: number;
}
