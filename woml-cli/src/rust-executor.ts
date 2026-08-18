import { existsSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';

import type { CompiledWorkflowDefinition, JsonObject, JsonValue } from '@woml/compiler';

import {
  decodeRunPresentationListV1,
  decodeRunPresentationV1,
  type RunPresentationListV1,
  type RunPresentationV1,
} from './terminal-presentation';
import {
  localNativeBinaryName,
  nativePackageName,
  nativeTargetForRuntime,
} from './native-platform';

export interface RustRunEvent {
  readonly eventSchemaVersion:
    | 1
    | 2
    | 3
    | 4
    | 5
    | 6
    | 7
    | 8
    | 9
    | 10
    | 11
    | 12;
  readonly eventId: string;
  readonly runId: string;
  readonly sequence: number;
  readonly occurredAt: string;
  readonly type: string;
  readonly data: unknown;
}

export interface RustWorkflowExecutionResult {
  readonly workflowId: string;
  readonly runId: string;
  readonly terminalNodeId: string;
  readonly result: JsonValue;
  readonly context: {
    readonly trigger: JsonObject;
    readonly steps: Readonly<Record<string, JsonValue>>;
  };
  readonly executionOrder: readonly string[];
  readonly events: readonly RustRunEvent[];
}

export interface RustExecutorOptions {
  readonly nativeCorePath?: string;
  readonly scriptHostPath?: string;
  readonly bunExecutable?: string;
  readonly scriptTimeoutMs?: number;
  readonly trigger?: JsonObject;
  readonly onProgress?: (progress: ExecutionProgressV1) => void;
  readonly resolvedSecrets?: Readonly<Record<string, string>>;
  readonly runtimeModules?: readonly RustRuntimeModuleArtifact[];
}

export interface RustRuntimeModuleArtifact {
  readonly name: string;
  readonly bundleDigest: string;
  readonly sourceMapDigest: string;
  readonly exports: readonly string[];
  readonly bundle: string;
  readonly sourceMap: string;
}

export type ExecutionProgressV1 =
  | {
      readonly contract: 'woml.execution-progress';
      readonly version: 1;
      readonly type: 'step_attempt_failed';
      readonly runId: string;
      readonly nodeId: string;
      readonly attempt: number;
      readonly maxAttempts: number;
      readonly failureCode: string;
    }
  | {
      readonly contract: 'woml.execution-progress';
      readonly version: 1;
      readonly type: 'step_retry_scheduled';
      readonly runId: string;
      readonly nodeId: string;
      readonly nextAttempt: number;
      readonly maxAttempts: number;
      readonly scheduledAt: string;
    }
  | {
      readonly contract: 'woml.execution-progress';
      readonly version: 1;
      readonly type: 'step_attempt_succeeded';
      readonly runId: string;
      readonly nodeId: string;
      readonly attempt: number;
      readonly maxAttempts: number;
    }
  | {
      readonly profile: 'woml.lifecycle-progress/v1';
      readonly runId: string;
      readonly workflowId: string;
      readonly phase:
        | 'hook_requested'
        | 'action_started'
        | 'action_succeeded'
        | 'action_failed'
        | 'hook_completed'
        | 'run_finalizing'
        | 'run_finalized';
      readonly hookId: string;
      readonly actionId: string;
      readonly stepId?: string;
      readonly code?: string;
    }
  | {
      readonly profile: 'woml.runtime-policy-progress/v1';
      readonly runId: string;
      readonly workflowId: string;
      readonly phase: 'queued' | 'eligible' | 'started' | 'timed_out';
      readonly queue: string;
      readonly waitingFor?: 'concurrency' | 'rate_limit';
      readonly eligibleAt?: string;
      readonly code?: 'WOML_WORKFLOW_TIMED_OUT';
    };

export interface RustRecoveryReport {
  readonly inspectedRuns: number;
  readonly recoveredRuns: number;
  readonly interruptedAttempts: number;
  readonly resumableRuns: number;
}

export interface StoredRunRequirementsV1 {
  readonly contract: 'woml.stored-run-requirements';
  readonly version: 1;
  readonly workflowId: string;
  readonly definitionHash: string;
  readonly requiredSecrets: readonly string[];
  readonly moduleCount: number;
  readonly hasApproval: boolean;
  readonly hasNotifications: boolean;
}

export type TriggerProgressV1 =
  | {
      readonly contract: 'woml.trigger-progress';
      readonly contractVersion: 1;
      readonly type: 'ready';
      readonly registrationCount: number;
      readonly occurredAt: string;
    }
  | {
      readonly contract: 'woml.trigger-progress';
      readonly contractVersion: 1;
      readonly type: 'occurrence_accepted';
      readonly workflowId: string;
      readonly triggerId: string;
      readonly triggerHandler: string;
      readonly occurrenceId: string;
      readonly runId: string;
      readonly duplicate: boolean;
      readonly occurredAt: string;
    }
  | {
      readonly contract: 'woml.trigger-progress';
      readonly contractVersion: 1;
      readonly type: 'run_started';
      readonly workflowId: string;
      readonly triggerId: string;
      readonly triggerHandler: string;
      readonly occurrenceId: string;
      readonly runId: string;
      readonly occurredAt: string;
    }
  | {
      readonly contract: 'woml.trigger-progress';
      readonly contractVersion: 1;
      readonly type: 'run_terminal';
      readonly workflowId: string;
      readonly runId: string;
      readonly status: 'succeeded' | 'failed';
      readonly failureCode?: string;
      readonly occurredAt: string;
    }
  | {
      readonly contract: 'woml.trigger-progress';
      readonly contractVersion: 1;
      readonly type: 'occurrence_rejected';
      readonly workflowId?: string;
      readonly triggerId?: string;
      readonly triggerHandler: string;
      readonly code: string;
      readonly message: string;
      readonly occurredAt: string;
    };

export type ScheduleProgressV1 =
  | {
      readonly contract: 'woml.schedule-progress';
      readonly contractVersion: 1;
      readonly type: 'next_due';
      readonly workflowId: string;
      readonly triggerId: string;
      readonly timezone: string;
      readonly nextScheduledAt: string;
      readonly reason:
        | 'initialized'
        | 'restarted'
        | 'advanced'
        | 'misfire_skipped'
        | 'misfire_run_once';
      readonly occurredAt: string;
    }
  | {
      readonly contract: 'woml.schedule-progress';
      readonly contractVersion: 1;
      readonly type: 'scheduler_error';
      readonly workflowId: string;
      readonly triggerId: string;
      readonly code: string;
      readonly message: string;
      readonly occurredAt: string;
    };

export type IntervalProgressV1 =
  | {
      readonly contract: 'woml.interval-progress';
      readonly contractVersion: 1;
      readonly type: 'next_due';
      readonly workflowId: string;
      readonly triggerId: string;
      readonly everyMs: number;
      readonly anchorAt: string;
      readonly nextSequence: number;
      readonly nextScheduledAt: string;
      readonly reason:
        | 'initialized'
        | 'restarted'
        | 'advanced'
        | 'misfire_skipped'
        | 'misfire_run_once';
      readonly occurredAt: string;
    }
  | {
      readonly contract: 'woml.interval-progress';
      readonly contractVersion: 1;
      readonly type: 'scheduler_error';
      readonly workflowId: string;
      readonly triggerId: string;
      readonly code: string;
      readonly message: string;
      readonly occurredAt: string;
    };

export type WorkflowCallProgressV1 =
  | {
      readonly contract: 'woml.workflow-call-progress';
      readonly contractVersion: 1;
      readonly type: 'call_admitted';
      readonly parentRunId: string;
      readonly parentNodeId: string;
      readonly targetWorkflowId: string;
      readonly childRunId: string;
      readonly duplicate: boolean;
      readonly occurredAt: string;
    }
  | {
      readonly contract: 'woml.workflow-call-progress';
      readonly contractVersion: 1;
      readonly type: 'child_terminal';
      readonly parentRunId: string;
      readonly targetWorkflowId: string;
      readonly childRunId: string;
      readonly status: 'succeeded' | 'failed';
      readonly occurredAt: string;
    }
  | {
      readonly contract: 'woml.workflow-call-progress';
      readonly contractVersion: 1;
      readonly type: 'call_rejected';
      readonly parentRunId: string;
      readonly parentNodeId: string;
      readonly targetWorkflowId: string;
      readonly code: string;
      readonly message: string;
      readonly occurredAt: string;
    };

export interface WebhookRuntimeRegistration {
  readonly workflow: CompiledWorkflowDefinition;
  readonly definitionHash: string;
  readonly resolvedSecrets: Readonly<Record<string, string>>;
  readonly runtimeModules?: readonly RustRuntimeModuleArtifact[];
}

export interface WebhookRuntimeOptions extends RustExecutorOptions {
  readonly host?: string;
  readonly port?: number;
  readonly startupManualTriggers?: Readonly<Record<string, string>>;
  readonly deploymentId?: string;
  readonly activationId?: string;
  readonly shutdownTimeoutMs?: number;
  /** Prepare and bind the runtime without opening trigger admission. */
  readonly startSuspended?: boolean;
  readonly onTriggerProgress?: (progress: TriggerProgressV1) => void;
  readonly onScheduleProgress?: (progress: ScheduleProgressV1) => void;
  readonly onIntervalProgress?: (progress: IntervalProgressV1) => void;
  readonly onWorkflowCallProgress?: (progress: WorkflowCallProgressV1) => void;
  readonly onRuntimePolicyProgress?: (
    progress: RuntimePolicyProgressV1
  ) => void;
  readonly onRuntimeLifecycle?: (progress: RuntimeInstanceV1) => void;
}

export interface RuntimeInstanceV1 {
  readonly profile: 'woml.runtime-instance/v1';
  readonly deploymentId: string;
  readonly activationId: string;
  readonly runtimeInstanceId: string;
  readonly runtimeVersion: string;
  readonly nativeVersion: string;
  readonly lifecycle:
    | 'starting'
    | 'recovering'
    | 'ready'
    | 'degraded'
    | 'draining'
    | 'stopped'
    | 'failed';
  readonly startedAt: string;
  readonly heartbeatAt: string;
  readonly leaseExpiresAt: string;
}

export type RuntimePolicyProgressV1 = Extract<
  ExecutionProgressV1,
  { readonly profile: 'woml.runtime-policy-progress/v1' }
>;

export interface WebhookRuntimeHandle {
  readonly runtimeId: string;
  readonly host: string;
  readonly port: number;
}

export interface TriggerIngressAdmit {
  readonly contract: 'woml.trigger-ingress';
  readonly contractVersion: 1;
  readonly messageType: 'admit';
  readonly requestId: string;
  readonly workflowId: string;
  readonly definitionHash: string;
  readonly triggerId: string;
  readonly triggerHandler:
    | 'trigger.slack'
    | 'trigger.telegram'
    | 'trigger.discord';
  readonly sourceIdentity: string;
  readonly payload: Readonly<Record<string, JsonValue>>;
  readonly receivedAt: string;
}

export type TriggerIngressOutcome =
  | {
      readonly contract: 'woml.trigger-ingress';
      readonly contractVersion: 1;
      readonly messageType: 'accepted';
      readonly requestId: string;
      readonly occurrenceId: string;
      readonly runId: string;
      readonly duplicate: boolean;
    }
  | {
      readonly contract: 'woml.trigger-ingress';
      readonly contractVersion: 1;
      readonly messageType: 'rejected';
      readonly requestId: string;
      readonly failure: {
        readonly code: string;
        readonly message: string;
        readonly retryable: boolean;
      };
    };

export interface ManualTriggerAdmissionRequestV1 {
  readonly profile: 'woml.manual-trigger-admission/v1';
  readonly type: 'request';
  readonly requestId: string;
  readonly workflowId: string;
  readonly triggerId: string;
  readonly payload: Readonly<Record<string, never>>;
  readonly requestedAt: string;
}

export type ManualTriggerAdmissionOutcomeV1 =
  | {
      readonly profile: 'woml.manual-trigger-admission/v1';
      readonly type: 'accepted';
      readonly requestId: string;
      readonly occurrenceId: string;
      readonly runId: string;
      readonly status: 'queued' | 'running';
    }
  | {
      readonly profile: 'woml.manual-trigger-admission/v1';
      readonly type: 'rejected';
      readonly requestId: string;
      readonly code:
        | 'WOML_MANUAL_TRIGGER_SELECTION_REQUIRED'
        | 'WOML_MANUAL_TRIGGER_ADMISSION_CLOSED'
        | 'WOML_POLICY_QUEUE_FULL';
      readonly message: string;
    };

export interface RustRunInspection {
  readonly runId: string;
  readonly workflowId: string;
  readonly status:
    | 'not_started'
    | 'running'
    | 'waiting'
    | 'cancelling'
    | 'finalizing'
    | 'succeeded'
    | 'failed'
    | 'cancelled';
  readonly terminalNodeId?: string;
  readonly result?: JsonValue;
  readonly failureCode?: string;
  readonly workflowCalls: {
    readonly parentCall?: RustWorkflowCallSummary;
    readonly childCalls: readonly RustWorkflowCallSummary[];
    readonly childCallsTruncated: boolean;
  };
}

export type PublicRunStatus =
  | 'not_started'
  | 'queued'
  | 'running'
  | 'waiting'
  | 'cancelling'
  | 'finalizing'
  | 'succeeded'
  | 'failed'
  | 'cancelled';

export interface RustRunSummaryV1 {
  readonly runId: string;
  readonly workflowId: string;
  readonly status: PublicRunStatus;
  readonly startedAt: string;
  readonly updatedAt: string;
}

export type RustRunListV1 =
  | {
      readonly profile: 'woml.run-list/v1';
      readonly runs: readonly RustRunSummaryV1[];
    }
  | {
      readonly profile: 'woml.run-list/v2';
      readonly runs: readonly {
        readonly runId: string;
        readonly workflowId: string;
        readonly status: PublicRunStatus;
        readonly admittedAt: string;
        readonly startedAt?: string;
        readonly updatedAt: string;
        readonly queue?: string;
        readonly waitingFor?: 'concurrency' | 'rate_limit';
        readonly eligibleAt?: string;
    }[];
    };

export interface RustRuntimeObservationV1 {
  readonly profile: 'woml.runtime-observation/v1';
  readonly statusTotals: Readonly<Record<string, number>>;
  readonly retryingRunIds: readonly string[];
  readonly approvalWaitingRunIds: readonly string[];
  readonly retriesTotal: number;
  readonly triggersTotal: number;
  readonly workflowCallsActive: number;
}

export interface RustBackupStoreInspection {
  readonly storeVersion: 13 | 14;
  readonly definitionHashes: readonly string[];
  readonly deploymentId?: string;
  readonly activationId?: string;
  readonly runtimeInstanceId?: string;
  readonly runtimeLeaseExpiresAt?: string;
}

export interface RetentionPolicyV1 {
  readonly policyId: string;
  readonly succeededBefore: string;
  readonly failedBefore: string;
  readonly cancelledBefore: string;
}

export interface RustRetentionPlanV1 extends RetentionPolicyV1 {
  readonly profile: 'woml.retention/v1';
  readonly kind: 'plan';
  readonly eligibleRuns: number;
  readonly estimatedBytes: number;
}

export interface RustRetentionResultV1 {
  readonly profile: 'woml.retention/v1';
  readonly kind: 'result';
  readonly policyId: string;
  readonly completedAt: string;
  readonly deletedRuns: number;
  readonly deletedBytes: number;
  readonly stateEntriesDeleted: 0;
}

export interface RustRetentionExecutionV1 {
  readonly result: RustRetentionResultV1;
  readonly batches: number;
  readonly checkpointBusy: number;
  readonly checkpointLogFrames: number;
  readonly checkpointedFrames: number;
  readonly compacted: boolean;
}

export interface RustLifecycleWarning {
  readonly hookId: string;
  readonly actionId: string;
  readonly stepId?: string;
  readonly provider?: string;
  readonly destination?: string;
  readonly code: string;
}

export interface RustRunInspectionV2 {
  readonly profile:
    | 'woml.run-inspection/v2'
    | 'woml.run-inspection/v3'
    | 'woml.run-inspection/v4'
    | 'woml.run-inspection/v5';
  readonly runId: string;
  readonly workflowId: string;
  readonly status: PublicRunStatus;
  readonly businessOutcome: 'undecided' | 'succeeded' | 'failed' | 'cancelled';
  readonly lifecycleStatus:
    | 'idle'
    | 'running'
    | 'finalizing'
    | 'completed_with_warnings'
    | 'completed';
  readonly hooks: readonly {
    readonly hookId: string;
    readonly subjectKind: 'workflow' | 'step';
    readonly subjectId: string;
    readonly status:
      | 'requested'
      | 'running'
      | 'completed'
      | 'completed_with_warnings';
    readonly failedActions: number;
  }[];
  readonly warnings: readonly RustLifecycleWarning[];
  readonly cancellation: {
    readonly requested: boolean;
    readonly requestId?: string;
  };
  readonly policy?: {
    readonly queue: string;
    readonly waitingFor?: 'concurrency' | 'rate_limit';
    readonly eligibleAt?: string;
    readonly timeoutAt?: string;
  };
  readonly forks?: Record<string, unknown>;
  readonly reusableDefinitions?: {
    readonly counts: {
      readonly pending: number;
      readonly running: number;
      readonly succeeded: number;
      readonly failed: number;
      readonly cancelled: number;
      readonly completedWithWarnings: number;
    };
    readonly items: readonly {
      readonly invocationId: string;
      readonly alias: string;
      readonly definitionDigestPrefix: string;
      readonly kind: 'step' | 'notification-provider';
      readonly status: 'pending' | 'running' | 'succeeded' | 'failed' | 'cancelled';
      readonly lifecycleStatus: 'idle' | 'running' | 'completed' | 'completed_with_warnings';
    }[];
  };
}

export interface RustRunCancellationResultV1 {
  readonly profile: 'woml.run-control.result/v1';
  readonly commandId: string;
  readonly runId: string;
  readonly status:
    | 'accepted'
    | 'already_requested'
    | 'already_cancelled'
    | 'rejected';
  readonly code?:
    | 'WOML_RUN_NOT_FOUND'
    | 'WOML_RUN_OUTCOME_ALREADY_DECIDED'
    | 'WOML_RUN_ALREADY_TERMINAL'
    | 'WOML_RUN_CONTROL_VERSION_UNSUPPORTED'
    | 'WOML_RUN_CANCELLATION_FAILED';
}

export interface RustWorkflowCallSummary {
  readonly parentRunId: string;
  readonly parentNodeId: string;
  readonly targetWorkflowId: string;
  readonly childRunId: string;
  readonly depth: number;
  readonly state: 'admitted' | 'running' | 'succeeded' | 'failed';
  readonly admittedAt: string;
}

export class TriggerRuntimeError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = 'TriggerRuntimeError';
    this.code = code;
  }
}

export class RunInspectionError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = 'RunInspectionError';
    this.code = code;
  }
}

export class RunManagementError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = 'RunManagementError';
    this.code = code;
  }
}

export class BackupOperationError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = 'BackupOperationError';
    this.code = code;
  }
}

export class RetentionOperationError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = 'RetentionOperationError';
    this.code = code;
  }
}

interface NativeExecutionErrorEnvelope {
  readonly kind: 'woml_execution_error';
  readonly code: string;
  readonly message: string;
  readonly nodeId?: string;
  readonly branchId?: string;
  readonly armId?: string;
  readonly referencePath?: readonly string[];
  readonly branchSite?: 'test' | 'result' | 'selection';
  readonly approvalId?: string;
  readonly requestId?: string;
  readonly attempt?: number;
  readonly maxAttempts?: number;
  readonly failureCode?: string;
  readonly details?: NativeParallelExecutionErrorDetails;
}

export interface NativeParallelExecutionErrorDetails {
  readonly parallelId: string;
  readonly policy: 'fail-fast' | 'wait-all';
  readonly primaryNodeId: string;
  readonly failedNodeIds: readonly string[];
  readonly cancelledNodeIds: readonly string[];
}

export class RustWorkflowExecutionError extends Error {
  readonly code: string;
  readonly nodeId?: string;
  readonly branchId?: string;
  readonly armId?: string;
  readonly referencePath?: readonly string[];
  readonly branchSite?: 'test' | 'result' | 'selection';
  readonly parallelId?: string;
  readonly parallelPolicy?: 'fail-fast' | 'wait-all';
  readonly primaryNodeId?: string;
  readonly failedNodeIds?: readonly string[];
  readonly cancelledNodeIds?: readonly string[];
  readonly approvalId?: string;
  readonly requestId?: string;
  readonly attempt?: number;
  readonly maxAttempts?: number;
  readonly failureCode?: string;

  constructor(
    code: string,
    message: string,
    details: {
      readonly nodeId?: string;
      readonly branchId?: string;
      readonly armId?: string;
      readonly referencePath?: readonly string[];
      readonly branchSite?: 'test' | 'result' | 'selection';
      readonly approvalId?: string;
      readonly requestId?: string;
      readonly attempt?: number;
      readonly maxAttempts?: number;
      readonly failureCode?: string;
      readonly parallel?: NativeParallelExecutionErrorDetails;
    } = {}
  ) {
    super(message);
    this.name = 'RustWorkflowExecutionError';
    this.code = code;
    if (details.nodeId !== undefined) this.nodeId = details.nodeId;
    if (details.branchId !== undefined) this.branchId = details.branchId;
    if (details.armId !== undefined) this.armId = details.armId;
    if (details.referencePath !== undefined) {
      this.referencePath = details.referencePath;
    }
    if (details.branchSite !== undefined) this.branchSite = details.branchSite;
    if (details.approvalId !== undefined) this.approvalId = details.approvalId;
    if (details.requestId !== undefined) this.requestId = details.requestId;
    if (details.attempt !== undefined) this.attempt = details.attempt;
    if (details.maxAttempts !== undefined)
      this.maxAttempts = details.maxAttempts;
    if (details.failureCode !== undefined)
      this.failureCode = details.failureCode;
    if (details.parallel !== undefined) {
      this.parallelId = details.parallel.parallelId;
      this.parallelPolicy = details.parallel.policy;
      this.primaryNodeId = details.parallel.primaryNodeId;
      this.failedNodeIds = details.parallel.failedNodeIds;
      this.cancelledNodeIds = details.parallel.cancelledNodeIds;
    }
  }
}

interface NativeCore {
  readonly executeWomlWorkflow: (
    compiledModelJson: string,
    definitionHash: string,
    triggerJson: string,
    bunExecutable: string,
    scriptHostPath: string,
    scriptTimeoutMs: number,
    resolvedSecretsJson: string,
    runtimeModulesJson?: string
  ) => Promise<string>;
  readonly executeWomlWorkflowDurable: (
    compiledModelJson: string,
    definitionHash: string,
    triggerJson: string,
    bunExecutable: string,
    scriptHostPath: string,
    scriptTimeoutMs: number,
    eventStorePath: string,
    resolvedSecretsJson: string,
    runtimeModulesJson?: string
  ) => Promise<string>;
  readonly executeWomlWorkflowDurableWithProgress: (
    compiledModelJson: string,
    definitionHash: string,
    triggerJson: string,
    bunExecutable: string,
    scriptHostPath: string,
    scriptTimeoutMs: number,
    eventStorePath: string,
    progressCallback: (message: string) => void,
    resolvedSecretsJson: string,
    runtimeModulesJson?: string
  ) => Promise<string>;
  readonly resumeWomlWorkflowDurableWithProgress: (
    compiledModelJson: string,
    definitionHash: string,
    runId: string,
    bunExecutable: string,
    scriptHostPath: string,
    scriptTimeoutMs: number,
    eventStorePath: string,
    progressCallback: (message: string) => void,
    resolvedSecretsJson: string,
    runtimeModulesJson?: string
  ) => Promise<string>;
  readonly recoverWomlRuns: (eventStorePath: string) => string;
  readonly startWomlWebhookRuntime: (
    registrationsJson: string,
    startupManualTriggersJson: string,
    bindAddress: string,
    eventStorePath: string,
    bunExecutable: string,
    scriptHostPath: string,
    scriptTimeoutMs: number,
    shutdownTimeoutMs: number,
    deploymentId: string,
    activationId: string,
    startSuspended: boolean,
    progressCallback: (message: string) => void
  ) => Promise<string>;
  readonly activateWomlWebhookRuntime: (runtimeId: string) => Promise<void>;
  readonly stopWomlWebhookRuntime: (runtimeId: string) => Promise<void>;
  readonly submitWomlTriggerOccurrence: (
    runtimeId: string,
    ingressJson: string
  ) => Promise<string>;
  readonly submitWomlManualTrigger: (
    runtimeId: string,
    requestJson: string
  ) => Promise<string>;
  readonly inspectWomlRun: (eventStorePath: string, runId: string) => string;
  readonly inspectWomlRunPresentation: (
    eventStorePath: string,
    runId: string
  ) => string;
  readonly listWomlRunPresentations: (
    eventStorePath: string,
    workflowId: string,
    limit: number
  ) => string;
  readonly hasWomlWorkflowDefinition: (
    eventStorePath: string,
    workflowId: string
  ) => boolean;
  readonly listWomlRuns: (
    eventStorePath: string,
    limit: number,
    workflowId?: string,
    status?: string
  ) => string;
  readonly observeWomlRuntime: (eventStorePath: string) => string;
  readonly createWomlBackup: (
    eventStorePath: string,
    destinationPath: string,
    leaseId: string,
    ownerId: string,
    fallbackDeploymentId: string
  ) => string;
  readonly inspectWomlBackupStore: (eventStorePath: string) => string;
  readonly recordWomlVerifiedBackup: (
    eventStorePath: string,
    backupId: string,
    completedAt: string
  ) => void;
  readonly prepareWomlRestoredStore: (
    eventStorePath: string,
    expectedDefinitionHashesJson: string,
    backupId: string,
    restoredAt: string
  ) => string;
  readonly planWomlRetention: (
    eventStorePath: string,
    policyJson: string,
    now: string
  ) => string;
  readonly executeWomlRetention: (
    eventStorePath: string,
    policyJson: string,
    leaseId: string,
    ownerId: string,
    compact: boolean,
    now: string
  ) => string;
  readonly executeWomlRetentionAsync?: (
    eventStorePath: string,
    policyJson: string,
    leaseId: string,
    ownerId: string,
    compact: boolean,
    now: string
  ) => Promise<string>;
  readonly readWomlLastRetentionResult: (eventStorePath: string) => string;
  readonly inspectWomlRunV2: (eventStorePath: string, runId: string) => string;
  readonly cancelWomlRun: (
    eventStorePath: string,
    runId: string,
    commandId: string
  ) => string;
  readonly inspectWomlStoredRunRequirements: (
    eventStorePath: string,
    runId: string
  ) => string;
  readonly resumeWomlStoredRunWithProgress: (
    runId: string,
    bunExecutable: string,
    scriptHostPath: string,
    scriptTimeoutMs: number,
    eventStorePath: string,
    progressCallback: (message: string) => void,
    resolvedSecretsJson: string
  ) => Promise<string>;
  readonly executeWomlWorkflowDurableOutcome: (
    compiledModelJson: string,
    definitionHash: string,
    triggerJson: string,
    bunExecutable: string,
    scriptHostPath: string,
    scriptTimeoutMs: number,
    eventStorePath: string,
    resolvedSecretsJson: string,
    runtimeModulesJson?: string
  ) => Promise<string>;
  readonly executeWomlWorkflowDurableOutcomeWithProgress: (
    compiledModelJson: string,
    definitionHash: string,
    triggerJson: string,
    bunExecutable: string,
    scriptHostPath: string,
    scriptTimeoutMs: number,
    eventStorePath: string,
    progressCallback: (message: string) => void,
    resolvedSecretsJson: string,
    runtimeModulesJson?: string
  ) => Promise<string>;
  readonly resumeWomlWorkflowDurableOutcome: (
    compiledModelJson: string,
    definitionHash: string,
    runId: string,
    bunExecutable: string,
    scriptHostPath: string,
    scriptTimeoutMs: number,
    eventStorePath: string,
    resolvedSecretsJson: string,
    runtimeModulesJson?: string
  ) => Promise<string>;
  readonly resumeWomlWorkflowDurableOutcomeWithProgress: (
    compiledModelJson: string,
    definitionHash: string,
    runId: string,
    bunExecutable: string,
    scriptHostPath: string,
    scriptTimeoutMs: number,
    eventStorePath: string,
    progressCallback: (message: string) => void,
    resolvedSecretsJson: string,
    runtimeModulesJson?: string
  ) => Promise<string>;
  readonly resolveWomlApproval: (
    eventStorePath: string,
    token: string,
    decision: ApprovalDecision
  ) => string;
  readonly resolveWomlNotificationApproval: (
    eventStorePath: string,
    capability: string,
    decision: ApprovalDecision,
    providerActorId?: string
  ) => string;
  readonly settleWomlApprovalTimeout: (
    eventStorePath: string,
    runId: string,
    approvalId: string
  ) => string;
  readonly runWomlNotificationProviderJourney: (
    eventStorePath: string,
    runId: string,
    bunExecutable: string,
    notificationHostPath: string,
    interactionTimeoutMs: number,
    customNotificationHostPath?: string,
    scriptHostPath?: string,
    approvalBaseUrl?: string,
    resolvedSecretsJson?: string
  ) => Promise<string>;
}

export type ApprovalDecision = 'approved' | 'rejected';

export interface WaitingApproval {
  readonly approvalId: string;
  readonly requestId: string;
  readonly name?: string;
  readonly description?: string;
  readonly expiresAt?: string;
  readonly onTimeout: 'reject' | 'fail';
  readonly token: string;
  readonly credentialExpiresAt: string;
}

export type RustApprovalRuntimeOutcome =
  | {
      readonly contract: 'woml.runtime-outcome';
      readonly version: 1;
      readonly status: 'succeeded';
      readonly execution: RustWorkflowExecutionResult;
    }
  | {
      readonly contract: 'woml.runtime-outcome';
      readonly version: 1;
      readonly status: 'waiting';
      readonly workflowId: string;
      readonly runId: string;
      readonly approval: WaitingApproval;
    };

export interface ApprovalDecisionResult {
  readonly contract: 'woml.approval-http';
  readonly version: 1;
  readonly status: 'accepted' | 'already_resolved';
  readonly runId: string;
  readonly approvalId: string;
  readonly requestId: string;
  readonly decision: ApprovalDecision;
  readonly source: 'human';
  readonly decidedAt: string;
}

export type ApprovalErrorCode =
  | 'WOML_APPROVAL_TOKEN_INVALID'
  | 'WOML_APPROVAL_TOKEN_EXPIRED'
  | 'WOML_APPROVAL_EXPIRED'
  | 'WOML_APPROVAL_DECISION_CONFLICT'
  | 'WOML_APPROVAL_INTERNAL';

export class ApprovalDecisionError extends Error {
  constructor(
    readonly code: ApprovalErrorCode,
    message: string
  ) {
    super(message);
    this.name = 'ApprovalDecisionError';
  }
}

export interface ApprovalTimeoutResult {
  readonly status: 'settled' | 'already_resolved' | 'not_due';
  readonly runId: string;
  readonly approvalId: string;
  readonly requestId: string;
  readonly resolution: unknown | null;
  readonly settledAt: string | null;
}

export interface NotificationDispatchReport {
  readonly attempted: number;
  readonly succeeded: number;
  readonly failed: number;
  readonly runFailed: boolean;
  readonly updatesAttempted: number;
  readonly updatesSucceeded: number;
  readonly updatesFailed: number;
}

export interface NotificationProviderJourneyResult {
  readonly runId: string;
  readonly decision: Omit<
    ApprovalDecisionResult,
    'contract' | 'version'
  > | null;
  readonly resolution: 'approved' | 'rejected' | 'timeout_failed';
  readonly deliveries: NotificationDispatchReport;
  readonly updates: NotificationDispatchReport;
  readonly diagnostics: NotificationJourneyDiagnostics;
}

export interface NotificationDeliveryFailureDiagnostic {
  readonly deliveryId: string;
  readonly provider: string;
  readonly destination: string;
  readonly attempt: number;
  readonly final: boolean;
  readonly failure: {
    readonly kind: string;
    readonly code: string;
    readonly message: string;
    readonly retryable: boolean;
    readonly retryAfterMs?: number;
  };
}

export interface NotificationJourneyDiagnostics {
  readonly version: 1;
  readonly deliveryFailures: readonly NotificationDeliveryFailureDiagnostic[];
}

export class NotificationProviderError extends Error {
  constructor(
    readonly code: string,
    message: string,
    readonly diagnostics?: NotificationJourneyDiagnostics
  ) {
    super(message);
    this.name = 'NotificationProviderError';
  }
}

function canonicalizeJson(value: unknown): string {
  if (
    value === null ||
    typeof value === 'boolean' ||
    typeof value === 'string'
  ) {
    return JSON.stringify(value);
  }
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) {
      throw new Error(
        'A compiled workflow definition must contain only finite JSON numbers.'
      );
    }
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) {
    return `[${value.map(canonicalizeJson).join(',')}]`;
  }
  if (typeof value === 'object') {
    const object = value as Readonly<Record<string, unknown>>;
    return `{${Object.keys(object)
      .sort()
      .map(key => `${JSON.stringify(key)}:${canonicalizeJson(object[key])}`)
      .join(',')}}`;
  }
  throw new Error('A compiled workflow definition must be strict JSON.');
}

export function compiledDefinitionHash(
  workflow: CompiledWorkflowDefinition
): string {
  const hexadecimal = new Bun.CryptoHasher('sha256')
    .update(canonicalizeJson(workflow))
    .digest('hex');
  return `sha256:${hexadecimal}`;
}

function defaultNativeCorePath(): string {
  const override = process.env.WOML_RUST_CORE_PATH;
  if (override !== undefined) return resolve(override);

  const local = resolve(
    import.meta.dir,
    localNativeBinaryName(process.platform, process.arch),
  );
  if (existsSync(local)) return local;

  const target = nativeTargetForRuntime(
    process.platform,
    process.arch,
  );
  const packageName = nativePackageName(target);
  try {
    return createRequire(import.meta.url).resolve(packageName);
  } catch {
    throw new Error(
      `WOML native package ${packageName} is unavailable. Reinstall woml on this machine and make sure optional dependencies are enabled.`,
    );
  }
}

function defaultScriptHostPath(): string {
  return resolve(
    import.meta.dir,
    import.meta.url.endsWith('.ts') ? 'script-host.ts' : 'script-host.js'
  );
}

function defaultNotificationHostPath(): string {
  return resolve(
    import.meta.dir,
    import.meta.url.endsWith('.ts')
      ? 'notification-provider-host.ts'
      : 'notification-provider-host.js'
  );
}

function defaultCustomNotificationHostPath(): string {
  return resolve(
    import.meta.dir,
    import.meta.url.endsWith('.ts')
      ? 'custom-notification-provider-host.ts'
      : 'custom-notification-provider-host.js'
  );
}

function loadNativeCore(path: string): NativeCore {
  const require = createRequire(import.meta.url);
  const loaded = require(path) as Partial<NativeCore>;
  if (typeof loaded.executeWomlWorkflow !== 'function') {
    throw new Error(
      `Native core at "${path}" does not expose executeWomlWorkflow; rebuild the Rust addon.`
    );
  }
  return loaded as NativeCore;
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

function dateTime(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    /^\d{4}-\d{2}-\d{2}T/.test(value) &&
    Number.isFinite(Date.parse(value))
  );
}

export function parseExecutionProgress(json: string): ExecutionProgressV1 {
  const value: unknown = JSON.parse(json);
  if (
    record(value) &&
    value.profile === 'woml.runtime-policy-progress/v1' &&
    exactKeys(
      value,
      ['profile', 'runId', 'workflowId', 'phase', 'queue'],
      ['waitingFor', 'eligibleAt', 'code']
    ) &&
    typeof value.runId === 'string' &&
    value.runId.length > 0 &&
    typeof value.workflowId === 'string' &&
    value.workflowId.length > 0 &&
    typeof value.queue === 'string' &&
    value.queue.length > 0 &&
    ['queued', 'eligible', 'started', 'timed_out'].includes(
      String(value.phase)
    ) &&
    (value.waitingFor === undefined ||
      ['concurrency', 'rate_limit'].includes(String(value.waitingFor))) &&
    (value.eligibleAt === undefined || dateTime(value.eligibleAt)) &&
    (value.code === undefined || value.code === 'WOML_WORKFLOW_TIMED_OUT')
  ) {
    return value as ExecutionProgressV1;
  }
  if (
    record(value) &&
    value.profile === 'woml.lifecycle-progress/v1' &&
    exactKeys(value, [
      'profile',
      'runId',
      'workflowId',
      'phase',
      'hookId',
      'actionId',
      ...(value.stepId === undefined ? [] : ['stepId']),
      ...(value.code === undefined ? [] : ['code']),
    ]) &&
    typeof value.runId === 'string' &&
    value.runId.length > 0 &&
    typeof value.workflowId === 'string' &&
    value.workflowId.length > 0 &&
    typeof value.hookId === 'string' &&
    value.hookId.length > 0 &&
    typeof value.actionId === 'string' &&
    value.actionId.length > 0 &&
    [
      'hook_requested',
      'action_started',
      'action_succeeded',
      'action_failed',
      'hook_completed',
      'run_finalizing',
      'run_finalized',
    ].includes(String(value.phase)) &&
    (value.stepId === undefined || typeof value.stepId === 'string') &&
    (value.code === undefined ||
      (typeof value.code === 'string' && /^WOML_[A-Z0-9_]+$/.test(value.code)))
  ) {
    return value as ExecutionProgressV1;
  }
  if (
    !record(value) ||
    value.contract !== 'woml.execution-progress' ||
    value.version !== 1 ||
    typeof value.runId !== 'string' ||
    value.runId.length === 0 ||
    typeof value.nodeId !== 'string' ||
    value.nodeId.length === 0 ||
    !Number.isSafeInteger(value.maxAttempts) ||
    Number(value.maxAttempts) < 1 ||
    Number(value.maxAttempts) > 10
  ) {
    throw new Error('The native core returned invalid execution progress.');
  }
  if (
    value.type === 'step_attempt_failed' &&
    exactKeys(value, [
      'contract',
      'version',
      'type',
      'runId',
      'nodeId',
      'attempt',
      'maxAttempts',
      'failureCode',
    ]) &&
    Number.isSafeInteger(value.attempt) &&
    Number(value.attempt) >= 1 &&
    Number(value.attempt) <= Number(value.maxAttempts) &&
    typeof value.failureCode === 'string' &&
    /^WOML_[A-Z0-9_]+$/.test(value.failureCode)
  ) {
    return value as ExecutionProgressV1;
  }
  if (
    value.type === 'step_retry_scheduled' &&
    exactKeys(value, [
      'contract',
      'version',
      'type',
      'runId',
      'nodeId',
      'nextAttempt',
      'maxAttempts',
      'scheduledAt',
    ]) &&
    Number.isSafeInteger(value.nextAttempt) &&
    Number(value.nextAttempt) >= 2 &&
    Number(value.nextAttempt) <= Number(value.maxAttempts) &&
    dateTime(value.scheduledAt)
  ) {
    return value as ExecutionProgressV1;
  }
  if (
    value.type === 'step_attempt_succeeded' &&
    exactKeys(value, [
      'contract',
      'version',
      'type',
      'runId',
      'nodeId',
      'attempt',
      'maxAttempts',
    ]) &&
    Number.isSafeInteger(value.attempt) &&
    Number(value.attempt) >= 1 &&
    Number(value.attempt) <= Number(value.maxAttempts)
  ) {
    return value as ExecutionProgressV1;
  }
  throw new Error('The native core returned invalid execution progress.');
}

export function parseTriggerProgress(json: string): TriggerProgressV1 {
  const value: unknown = JSON.parse(json);
  if (
    !record(value) ||
    value.contract !== 'woml.trigger-progress' ||
    value.contractVersion !== 1 ||
    !dateTime(value.occurredAt)
  ) {
    throw new Error('The native core returned invalid trigger progress.');
  }
  if (
    value.type === 'ready' &&
    exactKeys(value, [
      'contract',
      'contractVersion',
      'type',
      'registrationCount',
      'occurredAt',
    ]) &&
    Number.isSafeInteger(value.registrationCount) &&
    Number(value.registrationCount) >= 0
  ) {
    return value as TriggerProgressV1;
  }
  if (
    (value.type === 'occurrence_accepted' || value.type === 'run_started') &&
    exactKeys(
      value,
      [
        'contract',
        'contractVersion',
        'type',
        'workflowId',
        'triggerId',
        'triggerHandler',
        'occurrenceId',
        'runId',
        'occurredAt',
      ],
      value.type === 'occurrence_accepted' ? ['duplicate'] : []
    ) &&
    typeof value.workflowId === 'string' &&
    value.workflowId.length > 0 &&
    typeof value.triggerId === 'string' &&
    value.triggerId.length > 0 &&
    typeof value.triggerHandler === 'string' &&
    value.triggerHandler.startsWith('trigger.') &&
    typeof value.occurrenceId === 'string' &&
    value.occurrenceId.length > 0 &&
    typeof value.runId === 'string' &&
    value.runId.length > 0 &&
    (value.type !== 'occurrence_accepted' ||
      typeof value.duplicate === 'boolean')
  ) {
    return value as TriggerProgressV1;
  }
  if (
    value.type === 'run_terminal' &&
    exactKeys(
      value,
      [
        'contract',
        'contractVersion',
        'type',
        'workflowId',
        'runId',
        'status',
        'occurredAt',
      ],
      ['failureCode']
    ) &&
    typeof value.workflowId === 'string' &&
    value.workflowId.length > 0 &&
    typeof value.runId === 'string' &&
    value.runId.length > 0 &&
    (value.status === 'succeeded' || value.status === 'failed') &&
    (value.failureCode === undefined ||
      (typeof value.failureCode === 'string' && value.failureCode.length > 0))
  ) {
    return value as TriggerProgressV1;
  }
  if (
    value.type === 'occurrence_rejected' &&
    exactKeys(
      value,
      [
        'contract',
        'contractVersion',
        'type',
        'triggerHandler',
        'code',
        'message',
        'occurredAt',
      ],
      ['workflowId', 'triggerId']
    ) &&
    (value.workflowId === undefined || typeof value.workflowId === 'string') &&
    (value.triggerId === undefined || typeof value.triggerId === 'string') &&
    typeof value.triggerHandler === 'string' &&
    value.triggerHandler.startsWith('trigger.') &&
    typeof value.code === 'string' &&
    value.code.length > 0 &&
    typeof value.message === 'string' &&
    value.message.length > 0
  ) {
    return value as TriggerProgressV1;
  }
  throw new Error('The native core returned invalid trigger progress.');
}

export function parseWorkflowCallProgress(
  json: string
): WorkflowCallProgressV1 {
  const value: unknown = JSON.parse(json);
  if (
    !record(value) ||
    value.contract !== 'woml.workflow-call-progress' ||
    value.contractVersion !== 1 ||
    !dateTime(value.occurredAt) ||
    typeof value.parentRunId !== 'string' ||
    value.parentRunId.length === 0 ||
    typeof value.targetWorkflowId !== 'string' ||
    value.targetWorkflowId.length === 0
  ) {
    throw new Error('The native core returned invalid workflow call progress.');
  }
  if (
    value.type === 'call_admitted' &&
    exactKeys(value, [
      'contract',
      'contractVersion',
      'type',
      'parentRunId',
      'parentNodeId',
      'targetWorkflowId',
      'childRunId',
      'duplicate',
      'occurredAt',
    ]) &&
    typeof value.parentNodeId === 'string' &&
    value.parentNodeId.length > 0 &&
    typeof value.childRunId === 'string' &&
    value.childRunId.length > 0 &&
    typeof value.duplicate === 'boolean'
  ) {
    return value as WorkflowCallProgressV1;
  }
  if (
    value.type === 'child_terminal' &&
    exactKeys(value, [
      'contract',
      'contractVersion',
      'type',
      'parentRunId',
      'targetWorkflowId',
      'childRunId',
      'status',
      'occurredAt',
    ]) &&
    typeof value.childRunId === 'string' &&
    value.childRunId.length > 0 &&
    (value.status === 'succeeded' || value.status === 'failed')
  ) {
    return value as WorkflowCallProgressV1;
  }
  if (
    value.type === 'call_rejected' &&
    exactKeys(value, [
      'contract',
      'contractVersion',
      'type',
      'parentRunId',
      'parentNodeId',
      'targetWorkflowId',
      'code',
      'message',
      'occurredAt',
    ]) &&
    typeof value.parentNodeId === 'string' &&
    value.parentNodeId.length > 0 &&
    typeof value.code === 'string' &&
    value.code.startsWith('WOML_') &&
    typeof value.message === 'string' &&
    value.message.length > 0
  ) {
    return value as WorkflowCallProgressV1;
  }
  throw new Error('The native core returned invalid workflow call progress.');
}

export function parseScheduleProgress(json: string): ScheduleProgressV1 {
  const value: unknown = JSON.parse(json);
  if (
    !record(value) ||
    value.contract !== 'woml.schedule-progress' ||
    value.contractVersion !== 1 ||
    typeof value.workflowId !== 'string' ||
    value.workflowId.length === 0 ||
    typeof value.triggerId !== 'string' ||
    value.triggerId.length === 0 ||
    !dateTime(value.occurredAt)
  ) {
    throw new Error('The native core returned invalid schedule progress.');
  }
  if (
    value.type === 'next_due' &&
    exactKeys(value, [
      'contract',
      'contractVersion',
      'type',
      'workflowId',
      'triggerId',
      'timezone',
      'nextScheduledAt',
      'reason',
      'occurredAt',
    ]) &&
    typeof value.timezone === 'string' &&
    value.timezone.length > 0 &&
    dateTime(value.nextScheduledAt) &&
    [
      'initialized',
      'restarted',
      'advanced',
      'misfire_skipped',
      'misfire_run_once',
    ].includes(String(value.reason))
  ) {
    return value as ScheduleProgressV1;
  }
  if (
    value.type === 'scheduler_error' &&
    exactKeys(value, [
      'contract',
      'contractVersion',
      'type',
      'workflowId',
      'triggerId',
      'code',
      'message',
      'occurredAt',
    ]) &&
    typeof value.code === 'string' &&
    /^WOML_[A-Z0-9_]+$/.test(value.code) &&
    typeof value.message === 'string' &&
    value.message.length > 0
  ) {
    return value as ScheduleProgressV1;
  }
  throw new Error('The native core returned invalid schedule progress.');
}

export function parseIntervalProgress(json: string): IntervalProgressV1 {
  const value: unknown = JSON.parse(json);
  if (
    !record(value) ||
    value.contract !== 'woml.interval-progress' ||
    value.contractVersion !== 1 ||
    typeof value.workflowId !== 'string' ||
    value.workflowId.length === 0 ||
    typeof value.triggerId !== 'string' ||
    value.triggerId.length === 0 ||
    !dateTime(value.occurredAt)
  ) {
    throw new Error('The native core returned invalid interval progress.');
  }
  if (
    value.type === 'next_due' &&
    exactKeys(value, [
      'contract',
      'contractVersion',
      'type',
      'workflowId',
      'triggerId',
      'everyMs',
      'anchorAt',
      'nextSequence',
      'nextScheduledAt',
      'reason',
      'occurredAt',
    ]) &&
    Number.isSafeInteger(value.everyMs) &&
    Number(value.everyMs) >= 1_000 &&
    Number(value.everyMs) <= 2_592_000_000 &&
    dateTime(value.anchorAt) &&
    Number.isSafeInteger(value.nextSequence) &&
    Number(value.nextSequence) >= 1 &&
    dateTime(value.nextScheduledAt) &&
    [
      'initialized',
      'restarted',
      'advanced',
      'misfire_skipped',
      'misfire_run_once',
    ].includes(String(value.reason))
  ) {
    return value as IntervalProgressV1;
  }
  if (
    value.type === 'scheduler_error' &&
    exactKeys(value, [
      'contract',
      'contractVersion',
      'type',
      'workflowId',
      'triggerId',
      'code',
      'message',
      'occurredAt',
    ]) &&
    typeof value.code === 'string' &&
    /^WOML_[A-Z0-9_]+$/.test(value.code) &&
    typeof value.message === 'string' &&
    value.message.length > 0
  ) {
    return value as IntervalProgressV1;
  }
  throw new Error('The native core returned invalid interval progress.');
}

function executionResult(value: unknown): value is RustWorkflowExecutionResult {
  if (
    !record(value) ||
    !exactKeys(value, [
      'workflowId',
      'runId',
      'terminalNodeId',
      'result',
      'context',
      'executionOrder',
      'events',
    ]) ||
    typeof value.workflowId !== 'string' ||
    typeof value.runId !== 'string' ||
    typeof value.terminalNodeId !== 'string' ||
    !record(value.context) ||
    !exactKeys(value.context, ['trigger', 'steps']) ||
    !record(value.context.trigger) ||
    !record(value.context.steps) ||
    !Array.isArray(value.executionOrder) ||
    !value.executionOrder.every(nodeId => typeof nodeId === 'string') ||
    !Array.isArray(value.events)
  ) {
    return false;
  }
  return value.events.every(event => {
    if (!record(event)) return false;
    return (
      exactKeys(event, [
        'eventSchemaVersion',
        'eventId',
        'runId',
        'sequence',
        'occurredAt',
        'type',
        'data',
      ]) &&
      Number.isSafeInteger(event.eventSchemaVersion) &&
      Number(event.eventSchemaVersion) >= 1 &&
      Number(event.eventSchemaVersion) <= 13 &&
      typeof event.eventId === 'string' &&
      typeof event.runId === 'string' &&
      Number.isSafeInteger(event.sequence) &&
      dateTime(event.occurredAt) &&
      typeof event.type === 'string'
    );
  });
}

function waitingApproval(value: unknown): value is WaitingApproval {
  if (!record(value)) return false;
  return (
    exactKeys(
      value,
      ['approvalId', 'requestId', 'onTimeout', 'token', 'credentialExpiresAt'],
      ['name', 'description', 'expiresAt']
    ) &&
    /^[a-z][A-Za-z0-9]*$/.test(String(value.approvalId)) &&
    /^aprreq_[A-Za-z0-9_-]+$/.test(String(value.requestId)) &&
    (value.name === undefined ||
      (typeof value.name === 'string' && value.name.length > 0)) &&
    (value.description === undefined ||
      (typeof value.description === 'string' &&
        value.description.length > 0)) &&
    (value.expiresAt === undefined || dateTime(value.expiresAt)) &&
    (value.onTimeout === 'reject' || value.onTimeout === 'fail') &&
    /^apr_[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$/.test(String(value.token)) &&
    dateTime(value.credentialExpiresAt)
  );
}

function parseApprovalRuntimeOutcome(json: string): RustApprovalRuntimeOutcome {
  const value: unknown = JSON.parse(json);
  if (
    !record(value) ||
    value.contract !== 'woml.runtime-outcome' ||
    value.version !== 1
  ) {
    throw new Error('The native core returned an invalid approval outcome.');
  }
  if (
    value.status === 'succeeded' &&
    exactKeys(value, ['contract', 'version', 'status', 'execution']) &&
    executionResult(value.execution)
  ) {
    return value as unknown as RustApprovalRuntimeOutcome;
  }
  if (
    value.status === 'waiting' &&
    exactKeys(value, [
      'contract',
      'version',
      'status',
      'workflowId',
      'runId',
      'approval',
    ]) &&
    typeof value.workflowId === 'string' &&
    value.workflowId.length > 0 &&
    typeof value.runId === 'string' &&
    value.runId.length > 0 &&
    waitingApproval(value.approval)
  ) {
    return value as unknown as RustApprovalRuntimeOutcome;
  }
  throw new Error('The native core returned an invalid approval outcome.');
}

function parseApprovalDecisionResult(json: string): ApprovalDecisionResult {
  const value: unknown = JSON.parse(json);
  if (
    !record(value) ||
    !exactKeys(value, [
      'contract',
      'version',
      'status',
      'runId',
      'approvalId',
      'requestId',
      'decision',
      'source',
      'decidedAt',
    ]) ||
    value.contract !== 'woml.approval-http' ||
    value.version !== 1 ||
    (value.status !== 'accepted' && value.status !== 'already_resolved') ||
    typeof value.runId !== 'string' ||
    typeof value.approvalId !== 'string' ||
    typeof value.requestId !== 'string' ||
    (value.decision !== 'approved' && value.decision !== 'rejected') ||
    value.source !== 'human' ||
    !dateTime(value.decidedAt)
  ) {
    throw new Error('The native core returned an invalid approval decision.');
  }
  return value as unknown as ApprovalDecisionResult;
}

function requireApprovalMethods(native: NativeCore, path: string): void {
  for (const method of [
    'executeWomlWorkflowDurableOutcome',
    'resumeWomlWorkflowDurableOutcome',
    'resolveWomlApproval',
    'settleWomlApprovalTimeout',
  ] as const) {
    if (typeof native[method] !== 'function') {
      throw new Error(
        `Native core at "${path}" does not expose ${method}; rebuild the Rust addon.`
      );
    }
  }
}

function decodeNativeExecutionError(error: unknown): never {
  const message = error instanceof Error ? error.message : String(error);
  const jsonStart = message.indexOf('{');
  if (jsonStart !== -1) {
    try {
      const decoded = JSON.parse(
        message.slice(jsonStart)
      ) as Partial<NativeExecutionErrorEnvelope>;
      const parallelDetails = decoded.details;
      const validParallelDetails =
        parallelDetails === undefined ||
        (typeof parallelDetails === 'object' &&
          parallelDetails !== null &&
          typeof parallelDetails.parallelId === 'string' &&
          (parallelDetails.policy === 'fail-fast' ||
            parallelDetails.policy === 'wait-all') &&
          typeof parallelDetails.primaryNodeId === 'string' &&
          Array.isArray(parallelDetails.failedNodeIds) &&
          parallelDetails.failedNodeIds.every(
            nodeId => typeof nodeId === 'string'
          ) &&
          Array.isArray(parallelDetails.cancelledNodeIds) &&
          parallelDetails.cancelledNodeIds.every(
            nodeId => typeof nodeId === 'string'
          ));
      if (
        decoded.kind === 'woml_execution_error' &&
        typeof decoded.code === 'string' &&
        typeof decoded.message === 'string' &&
        (decoded.nodeId === undefined || typeof decoded.nodeId === 'string') &&
        (decoded.branchId === undefined ||
          typeof decoded.branchId === 'string') &&
        (decoded.armId === undefined || typeof decoded.armId === 'string') &&
        (decoded.referencePath === undefined ||
          (Array.isArray(decoded.referencePath) &&
            decoded.referencePath.every(part => typeof part === 'string'))) &&
        (decoded.branchSite === undefined ||
          decoded.branchSite === 'test' ||
          decoded.branchSite === 'result' ||
          decoded.branchSite === 'selection') &&
        (decoded.approvalId === undefined ||
          typeof decoded.approvalId === 'string') &&
        (decoded.requestId === undefined ||
          typeof decoded.requestId === 'string') &&
        (decoded.attempt === undefined ||
          (Number.isSafeInteger(decoded.attempt) &&
            Number(decoded.attempt) >= 1)) &&
        (decoded.maxAttempts === undefined ||
          (Number.isSafeInteger(decoded.maxAttempts) &&
            Number(decoded.maxAttempts) >= 1)) &&
        (decoded.failureCode === undefined ||
          (typeof decoded.failureCode === 'string' &&
            /^WOML_[A-Z0-9_]+$/.test(decoded.failureCode))) &&
        validParallelDetails
      ) {
        throw new RustWorkflowExecutionError(decoded.code, decoded.message, {
          nodeId: decoded.nodeId,
          branchId: decoded.branchId,
          armId: decoded.armId,
          referencePath: decoded.referencePath,
          branchSite: decoded.branchSite,
          approvalId: decoded.approvalId,
          requestId: decoded.requestId,
          attempt: decoded.attempt,
          maxAttempts: decoded.maxAttempts,
          failureCode: decoded.failureCode,
          parallel: parallelDetails,
        });
      }
    } catch (decodedError) {
      if (decodedError instanceof RustWorkflowExecutionError)
        throw decodedError;
    }
  }
  throw error;
}

function decodeNativeApprovalError(error: unknown): never {
  const message = error instanceof Error ? error.message : String(error);
  const jsonStart = message.indexOf('{');
  if (jsonStart !== -1) {
    try {
      const decoded: unknown = JSON.parse(message.slice(jsonStart));
      if (
        record(decoded) &&
        exactKeys(decoded, ['kind', 'code', 'message']) &&
        decoded.kind === 'woml_approval_error' &&
        typeof decoded.message === 'string' &&
        (decoded.code === 'WOML_APPROVAL_TOKEN_INVALID' ||
          decoded.code === 'WOML_APPROVAL_TOKEN_EXPIRED' ||
          decoded.code === 'WOML_APPROVAL_EXPIRED' ||
          decoded.code === 'WOML_APPROVAL_DECISION_CONFLICT' ||
          decoded.code === 'WOML_APPROVAL_INTERNAL')
      ) {
        throw new ApprovalDecisionError(decoded.code, decoded.message);
      }
    } catch (decodedError) {
      if (decodedError instanceof ApprovalDecisionError) throw decodedError;
    }
  }
  throw new ApprovalDecisionError(
    'WOML_APPROVAL_INTERNAL',
    'The approval decision could not be safely confirmed.'
  );
}

export async function executeWorkflowWithRust(
  workflow: CompiledWorkflowDefinition,
  options: RustExecutorOptions = {}
): Promise<RustWorkflowExecutionResult> {
  const timeoutMs = options.scriptTimeoutMs ?? 5_000;
  if (
    !Number.isSafeInteger(timeoutMs) ||
    timeoutMs < 1 ||
    timeoutMs > 0xffff_ffff
  ) {
    throw new Error('scriptTimeoutMs must be a positive 32-bit integer.');
  }

  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const scriptHostPath = options.scriptHostPath ?? defaultScriptHostPath();
  const bunExecutable = options.bunExecutable ?? process.execPath;
  const native = loadNativeCore(nativePath);
  let resultJson: string;
  try {
    resultJson = await native.executeWomlWorkflow(
      JSON.stringify(workflow),
      compiledDefinitionHash(workflow),
      JSON.stringify(options.trigger ?? {}),
      bunExecutable,
      scriptHostPath,
      timeoutMs,
      JSON.stringify(options.resolvedSecrets ?? {}),
      JSON.stringify(options.runtimeModules ?? [])
    );
  } catch (error) {
    decodeNativeExecutionError(error);
  }
  return JSON.parse(resultJson) as RustWorkflowExecutionResult;
}

export async function executeWorkflowWithRustDurable(
  workflow: CompiledWorkflowDefinition,
  eventStorePath: string,
  options: RustExecutorOptions = {}
): Promise<RustWorkflowExecutionResult> {
  const timeoutMs = options.scriptTimeoutMs ?? 5_000;
  if (
    !Number.isSafeInteger(timeoutMs) ||
    timeoutMs < 1 ||
    timeoutMs > 0xffff_ffff
  ) {
    throw new Error('scriptTimeoutMs must be a positive 32-bit integer.');
  }
  if (eventStorePath.length === 0) {
    throw new Error('eventStorePath must not be empty.');
  }

  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.executeWomlWorkflowDurable !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose executeWomlWorkflowDurable; rebuild the Rust addon.`
    );
  }
  const progressCallback = (message: string): void => {
    options.onProgress?.(parseExecutionProgress(message));
  };
  const execute =
    options.onProgress === undefined
      ? native.executeWomlWorkflowDurable
      : native.executeWomlWorkflowDurableWithProgress;
  if (typeof execute !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose executeWomlWorkflowDurableWithProgress; rebuild the Rust addon.`
    );
  }
  const arguments_ = [
    JSON.stringify(workflow),
    compiledDefinitionHash(workflow),
    JSON.stringify(options.trigger ?? {}),
    options.bunExecutable ?? process.execPath,
    options.scriptHostPath ?? defaultScriptHostPath(),
    timeoutMs,
    eventStorePath,
  ] as const;
  const secretsJson = JSON.stringify(options.resolvedSecrets ?? {});
  const runtimeModulesJson = JSON.stringify(options.runtimeModules ?? []);
  const resultJson = await (
    options.onProgress === undefined
      ? native.executeWomlWorkflowDurable(
          ...arguments_,
          secretsJson,
          runtimeModulesJson
        )
      : native.executeWomlWorkflowDurableWithProgress(
          ...arguments_,
          progressCallback,
          secretsJson,
          runtimeModulesJson
        )
  ).catch(decodeNativeExecutionError);
  return JSON.parse(resultJson) as RustWorkflowExecutionResult;
}

export async function resumeWorkflowWithRustDurable(
  workflow: CompiledWorkflowDefinition,
  eventStorePath: string,
  runId: string,
  options: RustExecutorOptions = {}
): Promise<RustWorkflowExecutionResult> {
  if (eventStorePath.length === 0 || runId.length === 0) {
    throw new Error('eventStorePath and runId must not be empty.');
  }
  const runtime = approvalRuntimeArguments(options);
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.resumeWomlWorkflowDurableWithProgress !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose resumeWomlWorkflowDurableWithProgress; rebuild the Rust addon.`
    );
  }
  const resultJson = await native
    .resumeWomlWorkflowDurableWithProgress(
      JSON.stringify(workflow),
      compiledDefinitionHash(workflow),
      runId,
      runtime.bunExecutable,
      runtime.scriptHostPath,
      runtime.timeoutMs,
      eventStorePath,
      message => options.onProgress?.(parseExecutionProgress(message)),
      JSON.stringify(options.resolvedSecrets ?? {}),
      JSON.stringify(options.runtimeModules ?? [])
    )
    .catch(decodeNativeExecutionError);
  return JSON.parse(resultJson) as RustWorkflowExecutionResult;
}

function approvalNative(options: RustExecutorOptions): {
  readonly native: NativeCore;
  readonly path: string;
} {
  const path = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(path);
  requireApprovalMethods(native, path);
  return { native, path };
}

function approvalRuntimeArguments(options: RustExecutorOptions): {
  readonly bunExecutable: string;
  readonly scriptHostPath: string;
  readonly timeoutMs: number;
} {
  const timeoutMs = options.scriptTimeoutMs ?? 5_000;
  if (
    !Number.isSafeInteger(timeoutMs) ||
    timeoutMs < 1 ||
    timeoutMs > 0xffff_ffff
  ) {
    throw new Error('scriptTimeoutMs must be a positive 32-bit integer.');
  }
  return {
    bunExecutable: options.bunExecutable ?? process.execPath,
    scriptHostPath: options.scriptHostPath ?? defaultScriptHostPath(),
    timeoutMs,
  };
}

export async function executeApprovalWorkflowWithRust(
  workflow: CompiledWorkflowDefinition,
  eventStorePath: string,
  options: RustExecutorOptions = {}
): Promise<RustApprovalRuntimeOutcome> {
  if (eventStorePath.length === 0) {
    throw new Error('eventStorePath must not be empty.');
  }
  const { native } = approvalNative(options);
  const runtime = approvalRuntimeArguments(options);
  const arguments_ = [
    JSON.stringify(workflow),
    compiledDefinitionHash(workflow),
    JSON.stringify(options.trigger ?? {}),
    runtime.bunExecutable,
    runtime.scriptHostPath,
    runtime.timeoutMs,
    eventStorePath,
  ] as const;
  const resultJson = await (
    options.onProgress === undefined
      ? native.executeWomlWorkflowDurableOutcome(
          ...arguments_,
          JSON.stringify(options.resolvedSecrets ?? {}),
          JSON.stringify(options.runtimeModules ?? [])
        )
      : native.executeWomlWorkflowDurableOutcomeWithProgress(
          ...arguments_,
          message => options.onProgress?.(parseExecutionProgress(message)),
          JSON.stringify(options.resolvedSecrets ?? {}),
          JSON.stringify(options.runtimeModules ?? [])
        )
  ).catch(decodeNativeExecutionError);
  return parseApprovalRuntimeOutcome(resultJson);
}

export async function resumeApprovalWorkflowWithRust(
  workflow: CompiledWorkflowDefinition,
  eventStorePath: string,
  runId: string,
  options: RustExecutorOptions = {}
): Promise<RustApprovalRuntimeOutcome> {
  if (eventStorePath.length === 0 || runId.length === 0) {
    throw new Error('eventStorePath and runId must not be empty.');
  }
  const { native } = approvalNative(options);
  const runtime = approvalRuntimeArguments(options);
  const arguments_ = [
    JSON.stringify(workflow),
    compiledDefinitionHash(workflow),
    runId,
    runtime.bunExecutable,
    runtime.scriptHostPath,
    runtime.timeoutMs,
    eventStorePath,
  ] as const;
  const resultJson = await (
    options.onProgress === undefined
      ? native.resumeWomlWorkflowDurableOutcome(
          ...arguments_,
          JSON.stringify(options.resolvedSecrets ?? {}),
          JSON.stringify(options.runtimeModules ?? [])
        )
      : native.resumeWomlWorkflowDurableOutcomeWithProgress(
          ...arguments_,
          message => options.onProgress?.(parseExecutionProgress(message)),
          JSON.stringify(options.resolvedSecrets ?? {}),
          JSON.stringify(options.runtimeModules ?? [])
        )
  ).catch(decodeNativeExecutionError);
  return parseApprovalRuntimeOutcome(resultJson);
}

export function resolveApprovalWithRust(
  eventStorePath: string,
  token: string,
  decision: ApprovalDecision,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): ApprovalDecisionResult {
  const { native } = approvalNative(options);
  try {
    return parseApprovalDecisionResult(
      native.resolveWomlApproval(eventStorePath, token, decision)
    );
  } catch (error) {
    if (error instanceof ApprovalDecisionError) throw error;
    decodeNativeApprovalError(error);
  }
}

export function resolveNotificationApprovalWithRust(
  eventStorePath: string,
  capability: string,
  decision: ApprovalDecision,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> & {
    readonly providerActorId?: string;
  } = {}
): ApprovalDecisionResult {
  const path = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(path);
  if (typeof native.resolveWomlNotificationApproval !== 'function') {
    throw new Error(
      `Native core at "${path}" does not expose resolveWomlNotificationApproval; rebuild the Rust addon.`
    );
  }
  try {
    return parseApprovalDecisionResult(
      native.resolveWomlNotificationApproval(
        eventStorePath,
        capability,
        decision,
        options.providerActorId
      )
    );
  } catch (error) {
    if (error instanceof ApprovalDecisionError) throw error;
    decodeNativeApprovalError(error);
  }
}

export function settleApprovalTimeoutWithRust(
  eventStorePath: string,
  runId: string,
  approvalId: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): ApprovalTimeoutResult {
  const { native } = approvalNative(options);
  let json: string;
  try {
    json = native.settleWomlApprovalTimeout(eventStorePath, runId, approvalId);
  } catch (error) {
    decodeNativeApprovalError(error);
  }
  const value: unknown = JSON.parse(json);
  if (
    !record(value) ||
    !exactKeys(value, [
      'status',
      'runId',
      'approvalId',
      'requestId',
      'resolution',
      'settledAt',
    ]) ||
    (value.status !== 'settled' &&
      value.status !== 'already_resolved' &&
      value.status !== 'not_due') ||
    typeof value.runId !== 'string' ||
    typeof value.approvalId !== 'string' ||
    typeof value.requestId !== 'string' ||
    (value.settledAt !== null && !dateTime(value.settledAt))
  ) {
    throw new Error('The native core returned an invalid approval timeout.');
  }
  return value as unknown as ApprovalTimeoutResult;
}

export function recoverDurableRuns(
  eventStorePath: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RustRecoveryReport {
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.recoverWomlRuns !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose recoverWomlRuns; rebuild the Rust addon.`
    );
  }
  return JSON.parse(
    native.recoverWomlRuns(eventStorePath)
  ) as RustRecoveryReport;
}

function decodeTriggerRuntimeError(error: unknown): never {
  if (error instanceof TriggerRuntimeError) throw error;
  const message = error instanceof Error ? error.message : String(error);
  try {
    const value: unknown = JSON.parse(message);
    if (
      record(value) &&
      value.kind === 'woml_trigger_runtime_error' &&
      typeof value.code === 'string' &&
      typeof value.message === 'string'
    ) {
      throw new TriggerRuntimeError(value.code, value.message);
    }
  } catch (decoded) {
    if (decoded instanceof TriggerRuntimeError) throw decoded;
  }
  throw new TriggerRuntimeError(
    'WOML_TRIGGER_RUNTIME_INTERNAL',
    'The native trigger runtime failed unexpectedly.'
  );
}

export async function startWebhookRuntimeWithRust(
  registrations: readonly WebhookRuntimeRegistration[],
  eventStorePath: string,
  options: WebhookRuntimeOptions = {}
): Promise<WebhookRuntimeHandle> {
  if (registrations.length === 0) {
    throw new Error('At least one webhook workflow registration is required.');
  }
  if (eventStorePath.length === 0) {
    throw new Error('eventStorePath must not be empty.');
  }
  const port = options.port ?? 3_000;
  if (!Number.isSafeInteger(port) || port < 0 || port > 65_535) {
    throw new Error(
      'Trigger runtime port must be an integer from 0 through 65535.'
    );
  }
  const timeoutMs = options.scriptTimeoutMs ?? 5_000;
  if (
    !Number.isSafeInteger(timeoutMs) ||
    timeoutMs < 1 ||
    timeoutMs > 0xffff_ffff
  ) {
    throw new Error('scriptTimeoutMs must be a positive 32-bit integer.');
  }
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  const shutdownTimeoutMs = options.shutdownTimeoutMs ?? 30_000;
  if (
    !Number.isSafeInteger(shutdownTimeoutMs) ||
    shutdownTimeoutMs < 1 ||
    shutdownTimeoutMs > 0xffff_ffff
  ) {
    throw new Error('shutdownTimeoutMs must be a positive 32-bit integer.');
  }
  const deploymentId = options.deploymentId ?? 'default';
  const activationId =
    options.activationId ?? compiledDefinitionHash(registrations[0]!.workflow);
  if (
    typeof native.startWomlWebhookRuntime !== 'function' ||
    typeof native.stopWomlWebhookRuntime !== 'function'
  ) {
    throw new Error(
      `Native core at "${nativePath}" does not expose the T4 webhook runtime; rebuild the Rust addon.`
    );
  }
  const resultJson = await native
    .startWomlWebhookRuntime(
      JSON.stringify(registrations),
      JSON.stringify(options.startupManualTriggers ?? {}),
      `${options.host ?? '127.0.0.1'}:${port}`,
      eventStorePath,
      options.bunExecutable ?? process.execPath,
      options.scriptHostPath ?? defaultScriptHostPath(),
      timeoutMs,
      shutdownTimeoutMs,
      deploymentId,
      activationId,
      options.startSuspended ?? false,
      message => {
        const decoded: unknown = JSON.parse(message);
        if (record(decoded) && decoded.contract === 'woml.schedule-progress') {
          options.onScheduleProgress?.(parseScheduleProgress(message));
          return;
        }
        if (record(decoded) && decoded.contract === 'woml.interval-progress') {
          options.onIntervalProgress?.(parseIntervalProgress(message));
          return;
        }
        if (
          record(decoded) &&
          decoded.contract === 'woml.workflow-call-progress'
        ) {
          options.onWorkflowCallProgress?.(parseWorkflowCallProgress(message));
          return;
        }
        if (
          record(decoded) &&
          decoded.profile === 'woml.runtime-policy-progress/v1'
        ) {
          const progress = parseExecutionProgress(message);
          if (
            'profile' in progress &&
            progress.profile === 'woml.runtime-policy-progress/v1'
          ) {
            options.onRuntimePolicyProgress?.(progress);
          }
          return;
        }
        if (
          record(decoded) &&
          decoded.profile === 'woml.runtime-instance/v1' &&
          typeof decoded.deploymentId === 'string' &&
          typeof decoded.activationId === 'string' &&
          typeof decoded.runtimeInstanceId === 'string' &&
          typeof decoded.runtimeVersion === 'string' &&
          typeof decoded.nativeVersion === 'string' &&
          [
            'starting',
            'recovering',
            'ready',
            'degraded',
            'draining',
            'stopped',
            'failed',
          ].includes(String(decoded.lifecycle)) &&
          typeof decoded.startedAt === 'string' &&
          typeof decoded.heartbeatAt === 'string' &&
          typeof decoded.leaseExpiresAt === 'string'
        ) {
          options.onRuntimeLifecycle?.(decoded as unknown as RuntimeInstanceV1);
          return;
        }
        options.onTriggerProgress?.(parseTriggerProgress(message));
      }
    )
    .catch(decodeTriggerRuntimeError);
  const value: unknown = JSON.parse(resultJson);
  if (
    !record(value) ||
    !exactKeys(value, ['runtimeId', 'host', 'port']) ||
    typeof value.runtimeId !== 'string' ||
    value.runtimeId.length === 0 ||
    typeof value.host !== 'string' ||
    value.host.length === 0 ||
    !Number.isSafeInteger(value.port) ||
    Number(value.port) < 0 ||
    Number(value.port) > 65_535
  ) {
    throw new Error(
      'The native core returned invalid webhook runtime startup data.'
    );
  }
  return value as unknown as WebhookRuntimeHandle;
}

export async function activateWebhookRuntimeWithRust(
  runtimeId: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): Promise<void> {
  if (runtimeId.length === 0) throw new Error('runtimeId must not be empty.');
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.activateWomlWebhookRuntime !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose atomic runtime activation; rebuild the Rust addon.`
    );
  }
  await native
    .activateWomlWebhookRuntime(runtimeId)
    .catch(decodeTriggerRuntimeError);
}

export async function submitTriggerOccurrenceWithRust(
  runtimeId: string,
  ingress: TriggerIngressAdmit,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): Promise<TriggerIngressOutcome> {
  if (runtimeId.length === 0) throw new Error('runtimeId must not be empty.');
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.submitWomlTriggerOccurrence !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose Slack trigger ingress; rebuild the Rust addon.`
    );
  }
  const resultJson = await native
    .submitWomlTriggerOccurrence(runtimeId, JSON.stringify(ingress))
    .catch(decodeTriggerRuntimeError);
  const value: unknown = JSON.parse(resultJson);
  if (
    !record(value) ||
    value.contract !== 'woml.trigger-ingress' ||
    value.contractVersion !== 1 ||
    value.requestId !== ingress.requestId
  ) {
    throw new Error('The native core returned invalid trigger ingress data.');
  }
  if (
    value.messageType === 'accepted' &&
    exactKeys(value, [
      'contract',
      'contractVersion',
      'messageType',
      'requestId',
      'occurrenceId',
      'runId',
      'duplicate',
    ]) &&
    typeof value.occurrenceId === 'string' &&
    value.occurrenceId.length > 0 &&
    typeof value.runId === 'string' &&
    value.runId.length > 0 &&
    typeof value.duplicate === 'boolean'
  ) {
    return value as unknown as TriggerIngressOutcome;
  }
  if (
    value.messageType === 'rejected' &&
    exactKeys(value, [
      'contract',
      'contractVersion',
      'messageType',
      'requestId',
      'failure',
    ]) &&
    record(value.failure) &&
    exactKeys(value.failure, ['code', 'message', 'retryable']) &&
    typeof value.failure.code === 'string' &&
    typeof value.failure.message === 'string' &&
    typeof value.failure.retryable === 'boolean'
  ) {
    return value as unknown as TriggerIngressOutcome;
  }
  throw new Error('The native core returned invalid trigger ingress data.');
}

export async function submitManualTriggerWithRust(
  runtimeId: string,
  request: ManualTriggerAdmissionRequestV1,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): Promise<ManualTriggerAdmissionOutcomeV1> {
  if (
    runtimeId.length === 0 ||
    request.profile !== 'woml.manual-trigger-admission/v1' ||
    request.type !== 'request' ||
    request.requestId.length === 0 ||
    request.requestId.length > 256 ||
    request.workflowId.length === 0 ||
    request.workflowId.length > 256 ||
    request.triggerId.length === 0 ||
    request.triggerId.length > 256 ||
    Object.keys(request.payload).length !== 0 ||
    !dateTime(request.requestedAt)
  ) {
    throw new Error('Manual Trigger Admission v1 request is invalid.');
  }
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.submitWomlManualTrigger !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose manual trigger admission; rebuild the Rust addon.`
    );
  }
  const resultJson = await native
    .submitWomlManualTrigger(runtimeId, JSON.stringify(request))
    .catch(decodeTriggerRuntimeError);
  const value: unknown = JSON.parse(resultJson);
  if (
    !record(value) ||
    value.profile !== 'woml.manual-trigger-admission/v1' ||
    value.requestId !== request.requestId
  ) {
    throw new Error('The native core returned invalid manual admission data.');
  }
  if (
    value.type === 'accepted' &&
    exactKeys(value, [
      'profile',
      'type',
      'requestId',
      'occurrenceId',
      'runId',
      'status',
    ]) &&
    typeof value.occurrenceId === 'string' &&
    value.occurrenceId.length > 0 &&
    typeof value.runId === 'string' &&
    /^run_[A-Za-z0-9_-]+$/.test(value.runId) &&
    ['queued', 'running'].includes(String(value.status))
  ) {
    return value as unknown as ManualTriggerAdmissionOutcomeV1;
  }
  if (
    value.type === 'rejected' &&
    exactKeys(value, [
      'profile',
      'type',
      'requestId',
      'code',
      'message',
    ]) &&
    [
      'WOML_MANUAL_TRIGGER_SELECTION_REQUIRED',
      'WOML_MANUAL_TRIGGER_ADMISSION_CLOSED',
      'WOML_POLICY_QUEUE_FULL',
    ].includes(String(value.code)) &&
    typeof value.message === 'string' &&
    value.message.length > 0 &&
    value.message.length <= 2_048
  ) {
    return value as unknown as ManualTriggerAdmissionOutcomeV1;
  }
  throw new Error('The native core returned invalid manual admission data.');
}

export async function stopWebhookRuntimeWithRust(
  runtimeId: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): Promise<void> {
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.stopWomlWebhookRuntime !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose the T4 webhook runtime; rebuild the Rust addon.`
    );
  }
  await native.stopWomlWebhookRuntime(runtimeId);
}

export function inspectRunWithRust(
  eventStorePath: string,
  runId: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RustRunInspection {
  if (eventStorePath.length === 0 || runId.length === 0) {
    throw new Error('eventStorePath and runId must not be empty.');
  }
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.inspectWomlRun !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose run inspection; rebuild the Rust addon.`
    );
  }
  let inspectionJson: string;
  try {
    inspectionJson = native.inspectWomlRun(eventStorePath, runId);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    try {
      const value: unknown = JSON.parse(message);
      if (
        record(value) &&
        value.kind === 'woml_run_inspection_error' &&
        typeof value.code === 'string' &&
        typeof value.message === 'string'
      ) {
        throw new RunInspectionError(value.code, value.message);
      }
    } catch (decoded) {
      if (decoded instanceof RunInspectionError) throw decoded;
    }
    throw new RunInspectionError(
      'WOML_RUN_INSPECTION_FAILED',
      'The durable WOML run could not be inspected.'
    );
  }
  const value: unknown = JSON.parse(inspectionJson);
  const workflowCallSummary = (summary: unknown): boolean =>
    record(summary) &&
    exactKeys(summary, [
      'parentRunId',
      'parentNodeId',
      'targetWorkflowId',
      'childRunId',
      'depth',
      'state',
      'admittedAt',
    ]) &&
    typeof summary.parentRunId === 'string' &&
    typeof summary.parentNodeId === 'string' &&
    typeof summary.targetWorkflowId === 'string' &&
    typeof summary.childRunId === 'string' &&
    Number.isSafeInteger(summary.depth) &&
    Number(summary.depth) >= 1 &&
    ['admitted', 'running', 'succeeded', 'failed'].includes(
      String(summary.state)
    ) &&
    dateTime(summary.admittedAt);
  if (
    !record(value) ||
    !exactKeys(
      value,
      ['runId', 'workflowId', 'status', 'workflowCalls'],
      ['terminalNodeId', 'result', 'failureCode']
    ) ||
    typeof value.runId !== 'string' ||
    typeof value.workflowId !== 'string' ||
    ![
      'not_started',
      'running',
      'waiting',
      'cancelling',
      'finalizing',
      'succeeded',
      'failed',
      'cancelled',
    ].includes(String(value.status)) ||
    (value.terminalNodeId !== undefined &&
      typeof value.terminalNodeId !== 'string') ||
    (value.failureCode !== undefined &&
      typeof value.failureCode !== 'string') ||
    !record(value.workflowCalls) ||
    !exactKeys(
      value.workflowCalls,
      ['childCalls', 'childCallsTruncated'],
      ['parentCall']
    ) ||
    !Array.isArray(value.workflowCalls.childCalls) ||
    value.workflowCalls.childCalls.length > 50 ||
    !value.workflowCalls.childCalls.every(workflowCallSummary) ||
    (value.workflowCalls.parentCall !== undefined &&
      !workflowCallSummary(value.workflowCalls.parentCall)) ||
    typeof value.workflowCalls.childCallsTruncated !== 'boolean'
  ) {
    throw new Error('The native core returned invalid run inspection data.');
  }
  return value as unknown as RustRunInspection;
}

export function inspectRunPresentationWithRust(
  eventStorePath: string,
  runId: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RunPresentationV1 {
  if (eventStorePath.length === 0 || runId.length === 0) {
    throw new Error('Run presentation requires a store path and run ID.');
  }
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.inspectWomlRunPresentation !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose Run Presentation v1; rebuild the Rust addon.`
    );
  }
  const value = callRunManagementNative(() =>
    native.inspectWomlRunPresentation(eventStorePath, runId)
  );
  return decodeRunPresentationV1(JSON.stringify(value));
}

export function listRunPresentationsWithRust(
  eventStorePath: string,
  workflowId: string,
  limit = 10,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RunPresentationListV1 {
  if (
    eventStorePath.length === 0 ||
    workflowId.length === 0 ||
    !Number.isSafeInteger(limit) ||
    limit < 1 ||
    limit > 10
  ) {
    throw new Error(
      'Recent run presentations require a store path, workflow ID, and a limit from 1 through 10.'
    );
  }
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.listWomlRunPresentations !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose Run Presentation listing; rebuild the Rust addon.`
    );
  }
  const value = callRunManagementNative(() =>
    native.listWomlRunPresentations(eventStorePath, workflowId, limit)
  );
  return decodeRunPresentationListV1(JSON.stringify(value));
}

export function hasWorkflowDefinitionWithRust(
  eventStorePath: string,
  workflowId: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): boolean {
  if (eventStorePath.length === 0 || workflowId.length === 0 || workflowId.length > 256) {
    throw new Error('Workflow definition lookup requires a store path and workflow ID.');
  }
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.hasWomlWorkflowDefinition !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose workflow definition lookup; rebuild the Rust addon.`
    );
  }
  return native.hasWomlWorkflowDefinition(eventStorePath, workflowId);
}

function callRunManagementNative(call: () => string): unknown {
  try {
    return JSON.parse(call());
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    try {
      const value: unknown = JSON.parse(message);
      if (
        record(value) &&
        value.kind === 'woml_run_management_error' &&
        typeof value.code === 'string' &&
        typeof value.message === 'string'
      ) {
        throw new RunManagementError(value.code, value.message);
      }
    } catch (decoded) {
      if (decoded instanceof RunManagementError) throw decoded;
    }
    throw new RunManagementError(
      'WOML_RUN_MANAGEMENT_FAILED',
      'The durable WOML run store could not complete the command.'
    );
  }
}

const PUBLIC_RUN_STATUSES: readonly PublicRunStatus[] = [
  'not_started',
  'queued',
  'running',
  'waiting',
  'cancelling',
  'finalizing',
  'succeeded',
  'failed',
  'cancelled',
];

export function listRunsWithRust(
  eventStorePath: string,
  filters: {
    readonly limit?: number;
    readonly workflowId?: string;
    readonly status?: PublicRunStatus;
  } = {},
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RustRunListV1 {
  const limit = filters.limit ?? 20;
  if (
    eventStorePath.length === 0 ||
    !Number.isSafeInteger(limit) ||
    limit < 1 ||
    limit > 200
  ) {
    throw new Error(
      'Run list requires a store path and a limit from 1 through 200.'
    );
  }
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.listWomlRuns !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose run listing; rebuild the Rust addon.`
    );
  }
  const value = callRunManagementNative(() =>
    native.listWomlRuns(
      eventStorePath,
      limit,
      filters.workflowId,
      filters.status
    )
  );
  const isV2 = record(value) && value.profile === 'woml.run-list/v2';
  const summary = (candidate: unknown): boolean =>
    record(candidate) &&
    (isV2
      ? exactKeys(
          candidate,
          ['runId', 'workflowId', 'status', 'admittedAt', 'updatedAt'],
          ['startedAt', 'queue', 'waitingFor', 'eligibleAt']
        )
      : exactKeys(candidate, [
          'runId',
          'workflowId',
          'status',
          'startedAt',
          'updatedAt',
        ])) &&
    typeof candidate.runId === 'string' &&
    typeof candidate.workflowId === 'string' &&
    PUBLIC_RUN_STATUSES.includes(candidate.status as PublicRunStatus) &&
    (isV2
      ? dateTime(candidate.admittedAt) &&
        (candidate.startedAt === undefined || dateTime(candidate.startedAt)) &&
        (candidate.queue === undefined ||
          typeof candidate.queue === 'string') &&
        (candidate.waitingFor === undefined ||
          ['concurrency', 'rate_limit'].includes(
            String(candidate.waitingFor)
          )) &&
        (candidate.eligibleAt === undefined || dateTime(candidate.eligibleAt))
      : dateTime(candidate.startedAt)) &&
    dateTime(candidate.updatedAt);
  if (
    !record(value) ||
    !exactKeys(value, ['profile', 'runs']) ||
    !['woml.run-list/v1', 'woml.run-list/v2'].includes(String(value.profile)) ||
    !Array.isArray(value.runs) ||
    value.runs.length > 200 ||
    !value.runs.every(summary)
  ) {
    throw new Error('The native core returned invalid run-list data.');
  }
  return value as unknown as RustRunListV1;
}

export function observeRuntimeWithRust(
  eventStorePath: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RustRuntimeObservationV1 {
  if (eventStorePath.length === 0) {
    throw new Error('Runtime observation requires a store path.');
  }
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.observeWomlRuntime !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose runtime observation; rebuild the Rust addon.`
    );
  }
  const value = callRunManagementNative(() =>
    native.observeWomlRuntime(eventStorePath)
  );
  const identifiers = (candidate: unknown): candidate is readonly string[] =>
    Array.isArray(candidate) &&
    candidate.length <= 1000 &&
    candidate.every(item => typeof item === 'string' && item.length > 0 && item.length <= 320);
  const totals = record(value) ? value.statusTotals : undefined;
  if (
    !record(value) ||
    !exactKeys(value, [
      'profile',
      'statusTotals',
      'retryingRunIds',
      'approvalWaitingRunIds',
      'retriesTotal',
      'triggersTotal',
      'workflowCallsActive',
    ]) ||
    value.profile !== 'woml.runtime-observation/v1' ||
    !record(totals) ||
    !Object.entries(totals).every(
      ([status, count]) =>
        PUBLIC_RUN_STATUSES.includes(status as PublicRunStatus) &&
        Number.isSafeInteger(count) &&
        Number(count) >= 0
    ) ||
    !identifiers(value.retryingRunIds) ||
    !identifiers(value.approvalWaitingRunIds) ||
    ![value.retriesTotal, value.triggersTotal, value.workflowCallsActive].every(
      count => Number.isSafeInteger(count) && Number(count) >= 0
    )
  ) {
    throw new Error('The native core returned invalid runtime-observation data.');
  }
  return value as unknown as RustRuntimeObservationV1;
}

function decodeBackupNativeError(error: unknown): never {
  const message = error instanceof Error ? error.message : String(error);
  try {
    const value: unknown = JSON.parse(message);
    if (
      record(value) &&
      value.kind === 'woml_backup_error' &&
      typeof value.code === 'string' &&
      typeof value.message === 'string'
    ) {
      throw new BackupOperationError(value.code, value.message);
    }
  } catch (decoded) {
    if (decoded instanceof BackupOperationError) throw decoded;
  }
  throw new BackupOperationError(
    'WOML_BACKUP_FAILED',
    'The native WOML backup operation failed.'
  );
}

function backupInspection(value: unknown): RustBackupStoreInspection {
  const digest = (candidate: unknown): candidate is string =>
    typeof candidate === 'string' && /^sha256:[0-9a-f]{64}$/.test(candidate);
  if (
    !record(value) ||
    !exactKeys(
      value,
      ['storeVersion', 'definitionHashes'],
      ['deploymentId', 'activationId', 'runtimeInstanceId', 'runtimeLeaseExpiresAt']
    ) ||
    ![13, 14].includes(Number(value.storeVersion)) ||
    !Array.isArray(value.definitionHashes) ||
    value.definitionHashes.length < 1 ||
    value.definitionHashes.length > 10_000 ||
    !value.definitionHashes.every(digest) ||
    new Set(value.definitionHashes).size !== value.definitionHashes.length ||
    (value.deploymentId !== undefined &&
      (typeof value.deploymentId !== 'string' || value.deploymentId.length === 0)) ||
    (value.activationId !== undefined && !digest(value.activationId)) ||
    (value.runtimeInstanceId !== undefined &&
      (typeof value.runtimeInstanceId !== 'string' || value.runtimeInstanceId.length === 0)) ||
    (value.runtimeLeaseExpiresAt !== undefined && !dateTime(value.runtimeLeaseExpiresAt))
  ) {
    throw new BackupOperationError(
      'WOML_BACKUP_VERIFICATION_FAILED',
      'The native core returned invalid backup inventory.'
    );
  }
  return value as unknown as RustBackupStoreInspection;
}

function backupJson(call: () => string): RustBackupStoreInspection {
  try {
    return backupInspection(JSON.parse(call()));
  } catch (error) {
    if (error instanceof BackupOperationError) throw error;
    decodeBackupNativeError(error);
  }
}

export function createBackupWithRust(
  eventStorePath: string,
  destinationPath: string,
  leaseId: string,
  ownerId: string,
  fallbackDeploymentId: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RustBackupStoreInspection {
  const native = loadNativeCore(options.nativeCorePath ?? defaultNativeCorePath());
  if (typeof native.createWomlBackup !== 'function') {
    throw new BackupOperationError(
      'WOML_BACKUP_UNAVAILABLE',
      'The native core does not expose production backup; rebuild the Rust addon.'
    );
  }
  return backupJson(() =>
    native.createWomlBackup(
      eventStorePath,
      destinationPath,
      leaseId,
      ownerId,
      fallbackDeploymentId
    )
  );
}

export function inspectBackupStoreWithRust(
  eventStorePath: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RustBackupStoreInspection {
  const native = loadNativeCore(options.nativeCorePath ?? defaultNativeCorePath());
  if (typeof native.inspectWomlBackupStore !== 'function') {
    throw new BackupOperationError(
      'WOML_BACKUP_UNAVAILABLE',
      'The native core does not expose backup verification; rebuild the Rust addon.'
    );
  }
  return backupJson(() => native.inspectWomlBackupStore(eventStorePath));
}

export function recordVerifiedBackupWithRust(
  eventStorePath: string,
  backupId: string,
  completedAt: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): void {
  const native = loadNativeCore(options.nativeCorePath ?? defaultNativeCorePath());
  if (typeof native.recordWomlVerifiedBackup !== 'function') {
    throw new BackupOperationError(
      'WOML_BACKUP_UNAVAILABLE',
      'The native core does not expose verified-backup recording; rebuild the Rust addon.'
    );
  }
  try {
    native.recordWomlVerifiedBackup(eventStorePath, backupId, completedAt);
  } catch (error) {
    decodeBackupNativeError(error);
  }
}

export function prepareRestoredStoreWithRust(
  eventStorePath: string,
  expectedDefinitionHashes: readonly string[],
  backupId: string,
  restoredAt: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RustBackupStoreInspection {
  const native = loadNativeCore(options.nativeCorePath ?? defaultNativeCorePath());
  if (typeof native.prepareWomlRestoredStore !== 'function') {
    throw new BackupOperationError(
      'WOML_RESTORE_UNAVAILABLE',
      'The native core does not expose production restore; rebuild the Rust addon.'
    );
  }
  return backupJson(() =>
    native.prepareWomlRestoredStore(
      eventStorePath,
      JSON.stringify(expectedDefinitionHashes),
      backupId,
      restoredAt
    )
  );
}

function decodeRetentionNativeError(error: unknown): never {
  const message = error instanceof Error ? error.message : String(error);
  try {
    const value: unknown = JSON.parse(message);
    if (
      record(value) &&
      value.kind === 'woml_retention_error' &&
      typeof value.code === 'string' &&
      typeof value.message === 'string'
    ) {
      throw new RetentionOperationError(value.code, value.message);
    }
  } catch (decoded) {
    if (decoded instanceof RetentionOperationError) throw decoded;
  }
  throw new RetentionOperationError(
    'WOML_RETENTION_FAILED',
    'The native WOML retention operation failed.'
  );
}

function count(value: unknown): value is number {
  return Number.isSafeInteger(value) && Number(value) >= 0;
}

function retentionResult(value: unknown): RustRetentionResultV1 {
  if (
    !record(value) ||
    !exactKeys(value, [
      'profile',
      'kind',
      'policyId',
      'completedAt',
      'deletedRuns',
      'deletedBytes',
      'stateEntriesDeleted',
    ]) ||
    value.profile !== 'woml.retention/v1' ||
    value.kind !== 'result' ||
    typeof value.policyId !== 'string' ||
    value.policyId.length < 1 ||
    value.policyId.length > 320 ||
    !dateTime(value.completedAt) ||
    !count(value.deletedRuns) ||
    !count(value.deletedBytes) ||
    value.stateEntriesDeleted !== 0
  ) {
    throw new RetentionOperationError(
      'WOML_RETENTION_RESULT_INVALID',
      'The native core returned an invalid Retention Result v1.'
    );
  }
  return value as unknown as RustRetentionResultV1;
}

export function planRetentionWithRust(
  eventStorePath: string,
  policy: RetentionPolicyV1,
  now: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RustRetentionPlanV1 {
  const native = loadNativeCore(options.nativeCorePath ?? defaultNativeCorePath());
  if (typeof native.planWomlRetention !== 'function') {
    throw new RetentionOperationError(
      'WOML_RETENTION_UNAVAILABLE',
      'The native core does not expose retention planning; rebuild the Rust addon.'
    );
  }
  try {
    const value: unknown = JSON.parse(
      native.planWomlRetention(eventStorePath, JSON.stringify(policy), now)
    );
    if (
      !record(value) ||
      !exactKeys(value, [
        'profile',
        'kind',
        'policyId',
        'succeededBefore',
        'failedBefore',
        'cancelledBefore',
        'eligibleRuns',
        'estimatedBytes',
      ]) ||
      value.profile !== 'woml.retention/v1' ||
      value.kind !== 'plan' ||
      typeof value.policyId !== 'string' ||
      !dateTime(value.succeededBefore) ||
      !dateTime(value.failedBefore) ||
      !dateTime(value.cancelledBefore) ||
      !count(value.eligibleRuns) ||
      !count(value.estimatedBytes)
    ) {
      throw new RetentionOperationError(
        'WOML_RETENTION_PLAN_INVALID',
        'The native core returned an invalid Retention Plan v1.'
      );
    }
    return value as unknown as RustRetentionPlanV1;
  } catch (error) {
    if (error instanceof RetentionOperationError) throw error;
    decodeRetentionNativeError(error);
  }
}

export function executeRetentionWithRust(
  eventStorePath: string,
  policy: RetentionPolicyV1,
  leaseId: string,
  ownerId: string,
  compact: boolean,
  now: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RustRetentionExecutionV1 {
  const native = loadNativeCore(options.nativeCorePath ?? defaultNativeCorePath());
  if (typeof native.executeWomlRetention !== 'function') {
    throw new RetentionOperationError(
      'WOML_RETENTION_UNAVAILABLE',
      'The native core does not expose retention execution; rebuild the Rust addon.'
    );
  }
  try {
    return decodeRetentionExecution(JSON.parse(
      native.executeWomlRetention(
        eventStorePath,
        JSON.stringify(policy),
        leaseId,
        ownerId,
        compact,
        now
      )
    ));
  } catch (error) {
    if (error instanceof RetentionOperationError) throw error;
    decodeRetentionNativeError(error);
  }
}

function decodeRetentionExecution(value: unknown): RustRetentionExecutionV1 {
  if (
    !record(value) ||
    !exactKeys(value, [
      'result',
      'batches',
      'checkpointBusy',
      'checkpointLogFrames',
      'checkpointedFrames',
      'compacted',
    ]) ||
    !count(value.batches) ||
    !count(value.checkpointBusy) ||
    !count(value.checkpointLogFrames) ||
    !count(value.checkpointedFrames) ||
    typeof value.compacted !== 'boolean'
  ) {
    throw new RetentionOperationError(
      'WOML_RETENTION_RESULT_INVALID',
      'The native core returned invalid retention execution metadata.'
    );
  }
  return {
    result: retentionResult(value.result),
    batches: value.batches,
    checkpointBusy: value.checkpointBusy,
    checkpointLogFrames: value.checkpointLogFrames,
    checkpointedFrames: value.checkpointedFrames,
    compacted: value.compacted,
  };
}

export async function executeRetentionWithRustAsync(
  eventStorePath: string,
  policy: RetentionPolicyV1,
  leaseId: string,
  ownerId: string,
  compact: boolean,
  now: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): Promise<RustRetentionExecutionV1> {
  const native = loadNativeCore(options.nativeCorePath ?? defaultNativeCorePath());
  if (typeof native.executeWomlRetentionAsync !== 'function') {
    throw new RetentionOperationError(
      'WOML_RETENTION_UNAVAILABLE',
      'The native core does not expose non-blocking retention execution; rebuild the Rust addon.'
    );
  }
  try {
    const encoded = await native.executeWomlRetentionAsync(
      eventStorePath,
      JSON.stringify(policy),
      leaseId,
      ownerId,
      compact,
      now
    );
    return decodeRetentionExecution(JSON.parse(encoded));
  } catch (error) {
    if (error instanceof RetentionOperationError) throw error;
    decodeRetentionNativeError(error);
  }
}

export function readLastRetentionResultWithRust(
  eventStorePath: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RustRetentionResultV1 | undefined {
  const native = loadNativeCore(options.nativeCorePath ?? defaultNativeCorePath());
  if (typeof native.readWomlLastRetentionResult !== 'function') return undefined;
  try {
    const value: unknown = JSON.parse(native.readWomlLastRetentionResult(eventStorePath));
    return value === null ? undefined : retentionResult(value);
  } catch (error) {
    if (error instanceof RetentionOperationError) throw error;
    decodeRetentionNativeError(error);
  }
}

export function inspectRunV2WithRust(
  eventStorePath: string,
  runId: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RustRunInspectionV2 {
  if (eventStorePath.length === 0 || runId.length === 0) {
    throw new Error('Run inspection requires a store path and run ID.');
  }
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.inspectWomlRunV2 !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose safe run inspection; rebuild the Rust addon.`
    );
  }
  const value = callRunManagementNative(() =>
    native.inspectWomlRunV2(eventStorePath, runId)
  );
  const hook = (candidate: unknown): boolean =>
    record(candidate) &&
    exactKeys(candidate, [
      'hookId',
      'subjectKind',
      'subjectId',
      'status',
      'failedActions',
    ]) &&
    typeof candidate.hookId === 'string' &&
    ['workflow', 'step'].includes(String(candidate.subjectKind)) &&
    typeof candidate.subjectId === 'string' &&
    ['requested', 'running', 'completed', 'completed_with_warnings'].includes(
      String(candidate.status)
    ) &&
    Number.isSafeInteger(candidate.failedActions) &&
    Number(candidate.failedActions) >= 0;
  const warning = (candidate: unknown): boolean =>
    record(candidate) &&
    exactKeys(
      candidate,
      ['hookId', 'actionId', 'code'],
      ['stepId', 'provider', 'destination']
    ) &&
    typeof candidate.hookId === 'string' &&
    typeof candidate.actionId === 'string' &&
    typeof candidate.code === 'string' &&
    (candidate.stepId === undefined || typeof candidate.stepId === 'string') &&
    (candidate.provider === undefined ||
      typeof candidate.provider === 'string') &&
    (candidate.destination === undefined ||
      typeof candidate.destination === 'string');
  const reusableDefinitions = (candidate: unknown): boolean =>
    record(candidate) &&
    exactKeys(candidate, ['counts', 'items']) &&
    record(candidate.counts) &&
    exactKeys(candidate.counts, [
      'pending', 'running', 'succeeded', 'failed', 'cancelled',
      'completedWithWarnings',
    ]) &&
    Object.values(candidate.counts).every(
      count => Number.isSafeInteger(count) && Number(count) >= 0
    ) &&
    Array.isArray(candidate.items) &&
    candidate.items.length <= 256 &&
    candidate.items.every(item =>
      record(item) &&
      exactKeys(item, [
        'invocationId', 'alias', 'definitionDigestPrefix', 'kind', 'status',
        'lifecycleStatus',
      ]) &&
      typeof item.invocationId === 'string' &&
      typeof item.alias === 'string' &&
      /^[a-f0-9]{12}$/.test(String(item.definitionDigestPrefix)) &&
      ['step', 'notification-provider'].includes(String(item.kind)) &&
      ['pending', 'running', 'succeeded', 'failed', 'cancelled'].includes(String(item.status)) &&
      ['idle', 'running', 'completed', 'completed_with_warnings'].includes(
        String(item.lifecycleStatus)
      )
    );
  if (
    !record(value) ||
    !exactKeys(
      value,
      [
        'profile',
        'runId',
        'workflowId',
        'status',
        'businessOutcome',
        'lifecycleStatus',
        'hooks',
        'warnings',
        'cancellation',
      ],
      ['policy', 'forks', 'reusableDefinitions']
    ) ||
    ![
      'woml.run-inspection/v2', 'woml.run-inspection/v3',
      'woml.run-inspection/v4', 'woml.run-inspection/v5',
    ].includes(
      String(value.profile)
    ) ||
    typeof value.runId !== 'string' ||
    typeof value.workflowId !== 'string' ||
    !PUBLIC_RUN_STATUSES.includes(value.status as PublicRunStatus) ||
    !['undecided', 'succeeded', 'failed', 'cancelled'].includes(
      String(value.businessOutcome)
    ) ||
    ![
      'idle',
      'running',
      'finalizing',
      'completed_with_warnings',
      'completed',
    ].includes(String(value.lifecycleStatus)) ||
    !Array.isArray(value.hooks) ||
    value.hooks.length > 1024 ||
    !value.hooks.every(hook) ||
    !Array.isArray(value.warnings) ||
    value.warnings.length > 128 ||
    !value.warnings.every(warning) ||
    !record(value.cancellation) ||
    !exactKeys(value.cancellation, ['requested'], ['requestId']) ||
    typeof value.cancellation.requested !== 'boolean' ||
    (value.cancellation.requestId !== undefined &&
      typeof value.cancellation.requestId !== 'string') ||
    (value.profile !== 'woml.run-inspection/v2' &&
      (!record(value.policy) ||
        !exactKeys(
          value.policy,
          ['queue'],
          ['waitingFor', 'eligibleAt', 'timeoutAt']
        ) ||
        typeof value.policy.queue !== 'string' ||
        (value.policy.waitingFor !== undefined &&
          !['concurrency', 'rate_limit'].includes(
            String(value.policy.waitingFor)
          )) ||
        (value.policy.eligibleAt !== undefined &&
          !dateTime(value.policy.eligibleAt)) ||
        (value.policy.timeoutAt !== undefined &&
          !dateTime(value.policy.timeoutAt)))) ||
    (value.profile === 'woml.run-inspection/v2' && value.policy !== undefined) ||
    (['woml.run-inspection/v4', 'woml.run-inspection/v5'].includes(String(value.profile)) &&
      !record(value.forks)) ||
    (value.profile === 'woml.run-inspection/v5' &&
      !reusableDefinitions(value.reusableDefinitions)) ||
    (value.profile !== 'woml.run-inspection/v5' && value.reusableDefinitions !== undefined)
  ) {
    throw new Error('The native core returned invalid run-inspection data.');
  }
  return value as unknown as RustRunInspectionV2;
}

export function cancelRunWithRust(
  eventStorePath: string,
  runId: string,
  commandId: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RustRunCancellationResultV1 {
  if (
    eventStorePath.length === 0 ||
    runId.length === 0 ||
    commandId.length === 0
  ) {
    throw new Error(
      'Run cancellation requires a store path, run ID, and command ID.'
    );
  }
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.cancelWomlRun !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose run cancellation; rebuild the Rust addon.`
    );
  }
  const value = callRunManagementNative(() =>
    native.cancelWomlRun(eventStorePath, runId, commandId)
  );
  const codes = [
    'WOML_RUN_NOT_FOUND',
    'WOML_RUN_OUTCOME_ALREADY_DECIDED',
    'WOML_RUN_ALREADY_TERMINAL',
    'WOML_RUN_CONTROL_VERSION_UNSUPPORTED',
    'WOML_RUN_CANCELLATION_FAILED',
  ];
  if (
    !record(value) ||
    !exactKeys(value, ['profile', 'commandId', 'runId', 'status'], ['code']) ||
    value.profile !== 'woml.run-control.result/v1' ||
    typeof value.commandId !== 'string' ||
    typeof value.runId !== 'string' ||
    ![
      'accepted',
      'already_requested',
      'already_cancelled',
      'rejected',
    ].includes(String(value.status)) ||
    (value.code !== undefined && !codes.includes(String(value.code)))
  ) {
    throw new Error('The native core returned invalid run-control data.');
  }
  return value as unknown as RustRunCancellationResultV1;
}

export function inspectStoredRunRequirementsWithRust(
  eventStorePath: string,
  runId: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): StoredRunRequirementsV1 {
  const path = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(path);
  if (typeof native.inspectWomlStoredRunRequirements !== 'function') {
    throw new Error(
      `Native core at "${path}" does not expose inspectWomlStoredRunRequirements; rebuild the Rust addon.`
    );
  }
  const decoded = JSON.parse(
    native.inspectWomlStoredRunRequirements(eventStorePath, runId)
  ) as StoredRunRequirementsV1;
  if (
    !record(decoded) ||
    !exactKeys(decoded, [
      'contract',
      'version',
      'workflowId',
      'definitionHash',
      'requiredSecrets',
      'moduleCount',
      'hasApproval',
      'hasNotifications',
    ]) ||
    decoded.contract !== 'woml.stored-run-requirements' ||
    decoded.version !== 1 ||
    typeof decoded.workflowId !== 'string' ||
    decoded.workflowId.length === 0 ||
    typeof decoded.definitionHash !== 'string' ||
    !/^sha256:[0-9a-f]{64}$/.test(decoded.definitionHash) ||
    !Array.isArray(decoded.requiredSecrets) ||
    !decoded.requiredSecrets.every(
      secret => typeof secret === 'string' && /^[A-Z][A-Z0-9_]*$/.test(secret)
    ) ||
    !Number.isSafeInteger(decoded.moduleCount) ||
    decoded.moduleCount < 0 ||
    decoded.moduleCount > 64 ||
    typeof decoded.hasApproval !== 'boolean' ||
    typeof decoded.hasNotifications !== 'boolean'
  ) {
    throw new Error(
      'The native core returned invalid stored-run requirements.'
    );
  }
  return decoded;
}

export async function resumeStoredRunWithRust(
  eventStorePath: string,
  runId: string,
  options: RustExecutorOptions = {}
): Promise<RustApprovalRuntimeOutcome> {
  const path = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(path);
  if (typeof native.resumeWomlStoredRunWithProgress !== 'function') {
    throw new Error(
      `Native core at "${path}" does not expose resumeWomlStoredRunWithProgress; rebuild the Rust addon.`
    );
  }
  const runtime = approvalRuntimeArguments(options);
  const json = await native
    .resumeWomlStoredRunWithProgress(
      runId,
      runtime.bunExecutable,
      runtime.scriptHostPath,
      runtime.timeoutMs,
      eventStorePath,
      message => options.onProgress?.(parseExecutionProgress(message)),
      JSON.stringify(options.resolvedSecrets ?? {})
    )
    .catch(decodeNativeExecutionError);
  return parseApprovalRuntimeOutcome(json);
}

function parseNotificationJourney(
  json: string
): NotificationProviderJourneyResult {
  const value: unknown = JSON.parse(json);
  if (
    !record(value) ||
    !exactKeys(value, [
      'runId',
      'decision',
      'resolution',
      'deliveries',
      'updates',
      'diagnostics',
    ]) ||
    typeof value.runId !== 'string' ||
    (value.decision !== null && !record(value.decision)) ||
    (value.resolution !== 'approved' &&
      value.resolution !== 'rejected' &&
      value.resolution !== 'timeout_failed') ||
    !record(value.deliveries) ||
    !record(value.updates) ||
    !notificationJourneyDiagnostics(value.diagnostics)
  ) {
    throw new Error(
      'The native core returned an invalid notification journey.'
    );
  }
  return value as unknown as NotificationProviderJourneyResult;
}

function notificationJourneyDiagnostics(
  value: unknown
): value is NotificationJourneyDiagnostics {
  const failureKinds = new Set([
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
    'script_threw',
    'timed_out',
    'cancelled',
    'non_json',
    'worker_crashed',
    'context_too_large',
    'result_too_large',
    'service_failed',
  ]);
  if (
    !record(value) ||
    !exactKeys(value, ['version', 'deliveryFailures']) ||
    value.version !== 1 ||
    !Array.isArray(value.deliveryFailures)
  ) {
    return false;
  }
  return value.deliveryFailures.every(item => {
    if (
      !record(item) ||
      !exactKeys(item, [
        'deliveryId',
        'provider',
        'destination',
        'attempt',
        'final',
        'failure',
      ]) ||
      typeof item.deliveryId !== 'string' ||
      !['slack', 'telegram', 'discord', 'whatsapp', 'custom'].includes(
        String(item.provider)
      ) ||
      typeof item.destination !== 'string' ||
      !Number.isSafeInteger(item.attempt) ||
      Number(item.attempt) < 1 ||
      Number(item.attempt) > 3 ||
      typeof item.final !== 'boolean' ||
      !record(item.failure) ||
      !exactKeys(
        item.failure,
        ['kind', 'code', 'message', 'retryable'],
        ['retryAfterMs']
      ) ||
      typeof item.failure.kind !== 'string' ||
      !failureKinds.has(item.failure.kind) ||
      typeof item.failure.code !== 'string' ||
      !/^WOML_[A-Z0-9_]+$/.test(item.failure.code) ||
      typeof item.failure.message !== 'string' ||
      item.failure.message.length < 1 ||
      item.failure.message.length > 1024 ||
      typeof item.failure.retryable !== 'boolean'
    ) {
      return false;
    }
    const marker =
      item.provider === 'telegram'
        ? 'chat'
        : item.provider === 'whatsapp'
          ? 'recipient'
          : 'channel';
    if (
      !new RegExp(
        `^[a-z][A-Za-z0-9]*:notify:(0|[1-9][0-9]*):${marker}:(0|[1-9][0-9]*)$`
      ).test(item.deliveryId) ||
      (item.provider === 'slack' &&
        !/^(#[a-z0-9][a-z0-9_-]{0,79}|[CG][A-Z0-9]{8,31})$/.test(
          item.destination
        )) ||
      (item.provider === 'telegram' &&
        !/^-?[0-9]{1,20}$/.test(item.destination)) ||
      (item.provider === 'discord' &&
        !/^[0-9]{17,20}$/.test(item.destination)) ||
      (item.provider === 'whatsapp' &&
        !/^[0-9]{8,16}$/.test(item.destination)) ||
      (item.provider === 'custom' &&
        !/^[a-z][a-z0-9-]{0,63}$/.test(item.destination))
    ) {
      return false;
    }
    return (
      item.failure.retryAfterMs === undefined ||
      (Number.isSafeInteger(item.failure.retryAfterMs) &&
        Number(item.failure.retryAfterMs) >= 0 &&
        Number(item.failure.retryAfterMs) <= 86_400_000)
    );
  });
}

function decodeNotificationError(error: unknown): never {
  const message = error instanceof Error ? error.message : String(error);
  const jsonStart = message.indexOf('{');
  if (jsonStart !== -1) {
    try {
      const value: unknown = JSON.parse(message.slice(jsonStart));
      if (
        record(value) &&
        exactKeys(value, ['kind', 'code', 'message'], ['diagnostics']) &&
        value.kind === 'woml_notification_error' &&
        typeof value.code === 'string' &&
        typeof value.message === 'string' &&
        (value.diagnostics === undefined ||
          notificationJourneyDiagnostics(value.diagnostics))
      ) {
        throw new NotificationProviderError(
          value.code,
          value.message,
          value.diagnostics
        );
      }
    } catch (decoded) {
      if (decoded instanceof NotificationProviderError) throw decoded;
    }
  }
  throw new NotificationProviderError(
    'WOML_NOTIFICATION_INTERNAL',
    'The notification provider journey could not be completed safely.'
  );
}

export async function runNotificationProviderJourneyWithRust(
  eventStorePath: string,
  runId: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath' | 'bunExecutable'> & {
    readonly notificationHostPath?: string;
    readonly customNotificationHostPath?: string;
    readonly scriptHostPath?: string;
    readonly approvalBaseUrl?: string;
    readonly resolvedSecrets?: Readonly<Record<string, string>>;
    readonly interactionTimeoutMs?: number;
  } = {}
): Promise<NotificationProviderJourneyResult> {
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.runWomlNotificationProviderJourney !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose runWomlNotificationProviderJourney; rebuild the Rust addon.`
    );
  }
  const timeoutMs = options.interactionTimeoutMs ?? 30_000;
  if (
    !Number.isSafeInteger(timeoutMs) ||
    timeoutMs < 1 ||
    timeoutMs > 0xffff_ffff
  ) {
    throw new Error('interactionTimeoutMs must be a positive 32-bit integer.');
  }
  const json = await native
    .runWomlNotificationProviderJourney(
      eventStorePath,
      runId,
      options.bunExecutable ?? process.execPath,
      options.notificationHostPath ?? defaultNotificationHostPath(),
      timeoutMs,
      options.customNotificationHostPath ??
        defaultCustomNotificationHostPath(),
      options.scriptHostPath ?? defaultScriptHostPath(),
      options.approvalBaseUrl,
      JSON.stringify(options.resolvedSecrets ?? {})
    )
    .catch(decodeNotificationError);
  return parseNotificationJourney(json);
}
