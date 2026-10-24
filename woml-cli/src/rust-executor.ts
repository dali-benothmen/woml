import { existsSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';

import type {
  CompiledWorkflowDefinition,
  JsonObject,
  JsonValue,
} from '@woml/compiler';

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
import { profileAsync, profileSync } from './performance-profiler';
import type { ProcessRuntimeAuthorityV1 } from './process-policy';
import type { ArtifactRuntimeAuthorityV1 } from './artifact-policy';
import type { InfrastructurePolicyRuntimeAuthorityV1 } from './infrastructure-policy';
import type {
  TelemetryAdmissionLinkV1,
  TelemetrySignalV1,
} from './opentelemetry-signal';
import {
  decodeOperationalObservationV1,
  decodeOperationalPageV1,
  decodeOperationalProgressV1,
  OperationalOutputError,
  parseOperationalOutputWakeup,
  type OperationalOutputObservationV1,
  type OperationalOutputWakeupV1,
  type OperationalPageV1,
  type OperationalProgressSnapshotV1,
} from './operational-output';

export interface ReviewerDirectoryV1 {
  readonly profile: 'woml.reviewer-directory/v1';
  readonly version: string;
  readonly principals: readonly {
    readonly id: string;
    readonly roles: readonly string[];
    readonly identities: readonly {
      readonly provider: 'slack' | 'telegram' | 'discord' | 'trusted-http';
      readonly accountId: string;
      readonly actorId: string;
    }[];
  }[];
}

export interface ReviewerRuntimeAuthorityV1 {
  readonly reviewers: { readonly directory: ReviewerDirectoryV1 };
}

export type RuntimeCapabilitiesV1 = Partial<
  ProcessRuntimeAuthorityV1 &
    ArtifactRuntimeAuthorityV1 &
    ReviewerRuntimeAuthorityV1 &
    InfrastructurePolicyRuntimeAuthorityV1
>;

export interface RustRunEvent {
  readonly eventSchemaVersion:
    1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 14 | 15;
  readonly eventId: string;
  readonly runId: string;
  readonly sequence: number;
  readonly occurredAt: string;
  readonly iteration?: {
    readonly forEachId: string;
    readonly index: number;
  };
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
  readonly onOperationalOutput?: (wakeup: OperationalOutputWakeupV1) => void;
  readonly resolvedSecrets?: Readonly<Record<string, string>>;
  readonly runtimeModules?: readonly RustRuntimeModuleArtifact[];
  readonly runtimeCapabilities?: RuntimeCapabilitiesV1;
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
      readonly type: 'for_each_progress';
      readonly runId: string;
      readonly forEachId: string;
      readonly status: 'running' | 'succeeded' | 'failed' | 'cancelled';
      readonly total: number;
      readonly succeeded: number;
      readonly failed: number;
      readonly skipped: number;
      readonly active: number;
      readonly pending: number;
      readonly concurrency: number;
    }
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
  /** Dedicated bearer token for the optional public external-signal endpoint. */
  readonly signalIngressToken?: string;
  readonly telemetryEnabled?: boolean;
  readonly telemetryPropagation?: boolean;
  readonly telemetrySamplingRatio?: number;
  readonly decodeTelemetrySignal?: (json: string) => TelemetrySignalV1;
  readonly decodeTelemetryAdmissionLink?: (
    json: string
  ) => TelemetryAdmissionLinkV1;
  readonly onTriggerProgress?: (progress: TriggerProgressV1) => void;
  readonly onScheduleProgress?: (progress: ScheduleProgressV1) => void;
  readonly onIntervalProgress?: (progress: IntervalProgressV1) => void;
  readonly onWorkflowCallProgress?: (progress: WorkflowCallProgressV1) => void;
  readonly onRuntimePolicyProgress?: (
    progress: RuntimePolicyProgressV1
  ) => void;
  readonly onRuntimeLifecycle?: (progress: RuntimeInstanceV1) => void;
  readonly onTelemetrySignal?: (signal: TelemetrySignalV1) => void;
  readonly onTelemetryAdmissionLink?: (link: TelemetryAdmissionLinkV1) => void;
}

export interface ExternalSignalAdmissionRequestV1 {
  readonly profile: 'woml.external-signal-admission/v1';
  readonly signal: string;
  readonly runId: string;
  readonly waitId: string;
  readonly correlation: string;
  readonly idempotencyKey: string;
  readonly payload: JsonValue;
  readonly receivedAt: string;
  readonly authenticationClass: 'local_control' | 'http_bearer';
}

export interface ExternalSignalOutcomeV1 {
  readonly profile: 'woml.signal-result/v1';
  readonly status:
    | 'accepted'
    | 'duplicate'
    | 'identity_conflict'
    | 'already_resolved'
    | 'target_not_found'
    | 'rejected';
  readonly signalId: string;
  readonly runId: string;
  readonly waitId: string;
  readonly resolution?: 'early' | 'resolved';
  readonly code?: string;
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
    'trigger.slack' | 'trigger.telegram' | 'trigger.discord';
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
  readonly storeVersion: 13 | 14 | 15 | 16 | 17 | 18 | 19 | 20;
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

export interface RustArtifactMaintenanceV1 {
  readonly profile: 'woml.artifact-maintenance/v1';
  readonly kind: 'plan' | 'result';
  readonly completedAt: string;
  readonly eligibleArtifacts: number;
  readonly eligibleBlobs: number;
  readonly estimatedBytes: number;
  readonly deletedArtifacts: number;
  readonly deletedBlobs: number;
  readonly deletedBytes: number;
  readonly scrubbedBlobs: number;
  readonly corruptBlobs: number;
  readonly collectedOrphans: number;
  readonly liveArtifacts: number;
  readonly liveBlobs: number;
  readonly liveBytes: number;
}

export interface RustArtifactBackupInventoryV1 {
  readonly profile: 'woml.artifact-backup-inventory/v1';
  readonly directory: 'artifacts';
  readonly blobCount: number;
  readonly totalBytes: number;
  readonly inventoryDigest: string;
  readonly verified: true;
}

export interface RustArtifactInspectionV1 {
  readonly profile: 'woml.artifact-inspection/v1';
  readonly artifact: {
    readonly contract: 'woml.artifact-reference';
    readonly contractVersion: 1;
    readonly id: string;
    readonly name: string;
    readonly size: number;
    readonly sha256: string;
    readonly contentType: string;
  };
  readonly digestPrefix: string;
  readonly status:
    'available' | 'quarantined' | 'expired' | 'corrupt' | 'deleted';
  readonly createdAt: string;
  readonly expiresAt: string;
  readonly retentionMs: number;
  readonly provenance: {
    readonly workflowId: string;
    readonly runId: string;
    readonly nodeId: string;
    readonly attempt: number;
  };
}

export interface RustArtifactRunInspectionV1 {
  readonly profile: 'woml.artifact-run-inspection/v1';
  readonly runId: string;
  readonly artifacts: readonly RustArtifactInspectionV1[];
  readonly truncated: boolean;
}

export interface RustArtifactExportV1 {
  readonly profile: 'woml.artifact-export/v1';
  readonly auditId: string;
  readonly artifactId: string;
  readonly name: string;
  readonly size: number;
  readonly digestPrefix: string;
  readonly exportedAt: string;
  readonly verified: true;
}

export interface RustArtifactObservationV1 {
  readonly profile: 'woml.artifact-observation/v1';
  readonly backend: 'local' | 's3';
  readonly backendHealthy: boolean;
  readonly transfers: number;
  readonly transferredBytes: number;
  readonly transferDurationMs: number;
  readonly failedTransfers: number;
  readonly corruptArtifacts: number;
  readonly orphanBlobs: number;
  readonly liveArtifacts: number;
  readonly liveBytes: number;
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
    | 'woml.run-inspection/v5'
    | 'woml.run-inspection/v6'
    | 'woml.run-inspection/v11';
  readonly runId: string;
  readonly workflowId: string;
  readonly status: PublicRunStatus;
  readonly businessOutcome: 'undecided' | 'succeeded' | 'failed' | 'cancelled';
  readonly lifecycleStatus:
    'idle' | 'running' | 'finalizing' | 'completed_with_warnings' | 'completed';
  readonly hooks: readonly {
    readonly hookId: string;
    readonly subjectKind: 'workflow' | 'step';
    readonly subjectId: string;
    readonly status:
      'requested' | 'running' | 'completed' | 'completed_with_warnings';
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
  readonly infrastructurePolicy?: {
    readonly profile: 'woml.infrastructure-policy-run-inspection/v1';
    readonly binding: {
      readonly runId: string;
      readonly policyId: string;
      readonly policyVersion: string;
      readonly policyDigest: string;
      readonly canonicalBundleReference: string;
      readonly signatureStatus: 'unsigned' | 'verified';
      readonly signingKeyId?: string;
      readonly admittedAt: string;
    };
    readonly decisions: readonly Record<string, unknown>[];
    readonly totalDecisions: number;
    readonly truncated: boolean;
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
      readonly status:
        'pending' | 'running' | 'succeeded' | 'failed' | 'cancelled';
      readonly lifecycleStatus:
        'idle' | 'running' | 'completed' | 'completed_with_warnings';
    }[];
  };
  readonly forEach?: {
    readonly counts: {
      readonly opened: number;
      readonly active: number;
      readonly succeeded: number;
      readonly failed: number;
      readonly cancelled: number;
    };
    readonly items: readonly {
      readonly forEachId: string;
      readonly status: 'active' | 'succeeded' | 'failed' | 'cancelled';
      readonly total: number;
      readonly succeeded: number;
      readonly failed: number;
      readonly skipped: number;
      readonly active: number;
      readonly pending: number;
      readonly concurrency: number;
      readonly failedIndex?: number;
      readonly failedNodeId?: string;
    }[];
  };
  readonly approvals?: {
    readonly pending: number;
    readonly truncated: boolean;
    readonly items: readonly {
      readonly approvalId: string;
      readonly requestId: string;
      readonly status: string;
      readonly reviewDigest: string;
      readonly review: JsonObject;
      readonly required: number;
      readonly received: number;
      readonly requiredRoles: readonly string[];
      readonly representedRoles: readonly string[];
      readonly directoryVersion: string;
      readonly requestedAt: string;
      readonly deadline?: string;
      readonly resolvedAt?: string;
      readonly decisions: readonly {
        readonly principalId: string;
        readonly decision: ApprovalDecision;
        readonly provider: string;
        readonly providerAccountId: string;
        readonly roles: readonly string[];
        readonly decidedAt: string;
      }[];
      readonly totalDecisions: number;
      readonly decisionsTruncated: boolean;
    }[];
  };
}

export interface RustWaitInspectionV1 {
  readonly profile: 'woml.wait-inspection/v1';
  readonly runId: string;
  readonly pendingWaits: number;
  readonly oldestPendingWaitAgeMs?: number;
  readonly waits: readonly {
    readonly runId: string;
    readonly waitId: string;
    readonly name: string;
    readonly kind: 'duration' | 'until' | 'signal';
    readonly status:
      'pending' | 'signaled' | 'elapsed' | 'timed_out' | 'cancelled';
    readonly registeredAt: string;
    readonly signals?: readonly string[];
    readonly correlationDigest?: string;
    readonly deadline?: string;
    readonly resolvedAt?: string;
    readonly winningSignal?: string;
  }[];
}

export interface RustCorrectionInspectionV1 {
  readonly profile: 'woml.correction-inspection/v1';
  readonly runId: string;
  readonly phase:
    'forward' | 'compensating' | 'cleanup' | 'completed' | 'manual_recovery';
  readonly originalOutcome: 'running' | 'succeeded' | 'failed' | 'cancelled';
  readonly sagas: readonly {
    readonly sagaId: string;
    readonly status: string;
    readonly registeredCompensations: number;
    readonly completedCompensations: number;
    readonly completedCleanup: number;
  }[];
  readonly unresolvedActions: readonly {
    readonly actionId: string;
    readonly kind: 'compensation' | 'cleanup';
    readonly code: string;
    readonly ambiguous: boolean;
  }[];
  readonly truncated: boolean;
}

export interface RustCorrectionRecoveryRequestV1 {
  readonly profile: 'woml.correction-recovery-request/v1';
  readonly runId: string;
  readonly action: 'retry' | 'externally_resolve';
  readonly actionId: string;
  readonly note?: string;
}

export interface RustCorrectionRecoveryResultV1 {
  readonly profile: 'woml.correction-recovery-result/v1';
  readonly runId: string;
  readonly actionId: string;
  readonly status: 'retry_scheduled' | 'externally_resolved' | 'rejected';
  readonly auditRecorded: true;
}

export interface RustCorrectionObservationV1 {
  readonly profile: 'woml.correction-observation/v1';
  readonly openSagas: number;
  readonly pendingActions: number;
  readonly activeAttempts: number;
  readonly manualRecoveryActions: number;
  readonly oldestManualRecoveryAgeMs?: number;
}

export interface RustLockInspectionV1 {
  readonly profile: 'woml.lock-inspection/v1';
  readonly runId: string;
  readonly lockId: string;
  readonly key: string;
  readonly status:
    'waiting' | 'held' | 'released' | 'timed_out' | 'cancelled' | 'lost';
  readonly registeredAt: string;
  readonly acquiredAt?: string;
  readonly releasedAt?: string;
  readonly deadline?: string;
  readonly expiresAt?: string;
  readonly fencingToken?: string;
  readonly waitedMs?: number;
  readonly heldMs?: number;
  readonly failureCode?: string;
}

export interface RustLockObservationV1 {
  readonly profile: 'woml.lock-observation/v1';
  readonly heldLocks: number;
  readonly waitingAcquisitions: number;
  readonly oldestWaiterAgeMs?: number;
  readonly renewalFailures: number;
  readonly expiredTakeovers: number;
  readonly acquisitionTimeouts: number;
}

export interface RustRunCancellationResultV1 {
  readonly profile: 'woml.run-control.result/v1';
  readonly commandId: string;
  readonly runId: string;
  readonly status:
    'accepted' | 'already_requested' | 'already_cancelled' | 'rejected';
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

export class ArtifactOperationError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = 'ArtifactOperationError';
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
    runtimeModulesJson?: string,
    runtimeCapabilitiesJson?: string
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
    runtimeModulesJson?: string,
    runtimeCapabilitiesJson?: string
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
    runtimeModulesJson?: string,
    runtimeCapabilitiesJson?: string
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
    runtimeModulesJson?: string,
    runtimeCapabilitiesJson?: string
  ) => Promise<string>;
  readonly recoverWomlRuns: (eventStorePath: string) => string;
  readonly settleWomlTimerWait?: (
    eventStorePath: string,
    runId: string,
    waitId: string
  ) => string;
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
    runtimeCapabilitiesJson: string | undefined,
    signalIngressToken: string | undefined,
    telemetryEnabled: boolean,
    telemetryPropagation: boolean,
    telemetrySamplingRatio: number,
    progressCallback: (message: string) => void
  ) => Promise<string>;
  readonly activateWomlWebhookRuntime: (runtimeId: string) => Promise<void>;
  readonly stopWomlWebhookRuntime: (runtimeId: string) => Promise<void>;
  readonly submitWomlTriggerOccurrence: (
    runtimeId: string,
    ingressJson: string
  ) => Promise<string>;
  readonly submitWomlExternalSignal?: (
    runtimeId: string,
    requestJson: string
  ) => Promise<string>;
  readonly submitWomlManualTrigger: (
    runtimeId: string,
    requestJson: string
  ) => Promise<string>;
  readonly inspectWomlRun: (eventStorePath: string, runId: string) => string;
  readonly inspectWomlRemoteDispatch?: (
    eventStorePath: string,
    dispatchId: string
  ) => string;
  readonly reconcileWomlRemoteDispatch?: (
    eventStorePath: string,
    dispatchId: string,
    resolution: string,
    actor: string,
    evidenceDigest: string
  ) => string;
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
  readonly readWomlOperationalOutputPage?: (
    eventStorePath: string,
    runId: string | undefined,
    after: number,
    limit: number
  ) => string;
  readonly inspectWomlOperationalProgress?: (
    eventStorePath: string,
    runId: string
  ) => string;
  readonly observeWomlOperationalOutput?: (eventStorePath: string) => string;
  readonly planWomlOperationalOutputPrune?: (
    eventStorePath: string,
    cutoff: string
  ) => string;
  readonly pruneWomlOperationalOutput?: (
    eventStorePath: string,
    cutoff: string
  ) => string;
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
  readonly maintainWomlArtifacts?: (
    eventStorePath: string,
    policyJson: string,
    modeJson: string,
    resolvedSecretsJson: string,
    now: string,
    dryRun: boolean
  ) => string;
  readonly inspectWomlArtifact?: (
    eventStorePath: string,
    artifactId: string
  ) => string;
  readonly inspectWomlRunArtifacts?: (
    eventStorePath: string,
    runId: string
  ) => string;
  readonly exportWomlArtifact?: (
    eventStorePath: string,
    destinationPath: string,
    artifactId: string,
    policyJson: string,
    modeJson: string,
    resolvedSecretsJson: string
  ) => string;
  readonly observeWomlArtifacts?: (
    eventStorePath: string,
    policyJson: string,
    modeJson: string,
    resolvedSecretsJson: string
  ) => string;
  readonly exportWomlArtifactBackup?: (
    eventStorePath: string,
    destinationPath: string,
    policyJson: string,
    modeJson: string,
    resolvedSecretsJson: string
  ) => string;
  readonly restoreWomlArtifactBackup?: (
    eventStorePath: string,
    sourcePath: string,
    policyJson: string,
    modeJson: string,
    resolvedSecretsJson: string
  ) => string;
  readonly verifyWomlArtifactBackup?: (
    eventStorePath: string,
    sourcePath: string
  ) => string;
  readonly inspectWomlRunV2: (eventStorePath: string, runId: string) => string;
  readonly inspectWomlWaitsV1: (
    eventStorePath: string,
    runId: string
  ) => string;
  readonly inspectWomlCorrectionV1?: (
    eventStorePath: string,
    runId: string
  ) => string;
  readonly recoverWomlCorrection?: (
    eventStorePath: string,
    requestJson: string
  ) => string;
  readonly observeWomlCorrectionsV1?: (eventStorePath: string) => string;
  readonly inspectWomlLocksV1?: (
    eventStorePath: string,
    runId: string
  ) => string;
  readonly observeWomlLocksV1?: (eventStorePath: string) => string;
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
    resolvedSecretsJson: string,
    runtimeCapabilitiesJson?: string
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
    runtimeModulesJson?: string,
    runtimeCapabilitiesJson?: string
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
    runtimeModulesJson?: string,
    runtimeCapabilitiesJson?: string
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
    runtimeModulesJson?: string,
    runtimeCapabilitiesJson?: string
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
    runtimeModulesJson?: string,
    runtimeCapabilitiesJson?: string
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
  readonly resolveWomlRichApproval: (
    eventStorePath: string,
    runId: string,
    approvalId: string,
    requestId: string,
    reviewDigest: string,
    decision: ApprovalDecision,
    provider: 'slack' | 'telegram' | 'discord' | 'trusted-http',
    providerAccountId: string,
    providerActorId: string
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
  readonly token?: string;
  readonly credentialExpiresAt?: string;
  readonly reviewDigest?: string;
  readonly review?: JsonObject;
  readonly authority?: {
    readonly required: number;
    readonly received: number;
    readonly requiredRoles: readonly string[];
    readonly representedRoles: readonly string[];
  };
}

export interface WaitingTimer {
  readonly waitId: string;
  readonly nodeId: string;
  readonly kind: 'duration' | 'until';
  readonly registeredAt: string;
  readonly deadline: string;
}

export interface WaitingSignal {
  readonly waitId: string;
  readonly nodeId: string;
  readonly registeredAt: string;
  readonly signals: readonly string[];
  readonly deadline?: string;
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
    }
  | {
      readonly contract: 'woml.runtime-outcome';
      readonly version: 1;
      readonly status: 'waiting_time';
      readonly workflowId: string;
      readonly runId: string;
      readonly wait: WaitingTimer;
    }
  | {
      readonly contract: 'woml.runtime-outcome';
      readonly version: 1;
      readonly status: 'waiting_signal';
      readonly workflowId: string;
      readonly runId: string;
      readonly wait: WaitingSignal;
    };

export type TimerWaitSettlement =
  | { readonly status: 'resolved' | 'already_resolved' }
  | { readonly status: 'not_due'; readonly deadline: string };

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

export interface RichApprovalDecisionResult {
  readonly contract: 'woml.approval-http';
  readonly version: 2;
  readonly status:
    'recorded' | 'resolved' | 'already_recorded' | 'already_resolved';
  readonly runId: string;
  readonly approvalId: string;
  readonly requestId: string;
  readonly reviewDigest: string;
}

export type ApprovalErrorCode =
  | 'WOML_APPROVAL_TOKEN_INVALID'
  | 'WOML_APPROVAL_TOKEN_EXPIRED'
  | 'WOML_APPROVAL_EXPIRED'
  | 'WOML_APPROVAL_DECISION_CONFLICT'
  | 'WOML_APPROVAL_BINDING_MISMATCH'
  | 'WOML_APPROVAL_REVIEWER_UNKNOWN'
  | 'WOML_APPROVAL_REVIEWER_FORBIDDEN'
  | 'WOML_APPROVAL_DUPLICATE_REVIEWER'
  | 'WOML_APPROVAL_AUTHORITY_UNAVAILABLE'
  | 'WOML_APPROVAL_ALREADY_RESOLVED'
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
  return profileSync('compiler', 'compiler.hash_definition', measurements => {
    const canonical = canonicalizeJson(workflow);
    if (measurements !== undefined) {
      measurements.bytes.definition = Buffer.byteLength(canonical);
    }
    const hexadecimal = new Bun.CryptoHasher('sha256')
      .update(canonical)
      .digest('hex');
    return `sha256:${hexadecimal}`;
  });
}

function defaultNativeCorePath(): string {
  const override = process.env.WOML_RUST_CORE_PATH;
  if (override !== undefined) return resolve(override);

  const local = resolve(
    import.meta.dir,
    localNativeBinaryName(process.platform, process.arch)
  );
  if (existsSync(local)) return local;

  const target = nativeTargetForRuntime(process.platform, process.arch);
  const packageName = nativePackageName(target);
  try {
    return createRequire(import.meta.url).resolve(packageName);
  } catch {
    throw new Error(
      `WOML native package ${packageName} is unavailable. Reinstall woml on this machine and make sure optional dependencies are enabled.`
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
  return profileSync('napi', 'napi.load_native_addon', () => {
    const require = createRequire(import.meta.url);
    const loaded = require(path) as Partial<NativeCore>;
    if (typeof loaded.executeWomlWorkflow !== 'function') {
      throw new Error(
        `Native core at "${path}" does not expose executeWomlWorkflow; rebuild the Rust addon.`
      );
    }
    return loaded as NativeCore;
  });
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
    record(value) &&
    value.contract === 'woml.execution-progress' &&
    value.version === 1 &&
    value.type === 'for_each_progress' &&
    exactKeys(value, [
      'contract',
      'version',
      'type',
      'runId',
      'forEachId',
      'status',
      'total',
      'succeeded',
      'failed',
      'skipped',
      'active',
      'pending',
      'concurrency',
    ]) &&
    typeof value.runId === 'string' &&
    value.runId.length > 0 &&
    typeof value.forEachId === 'string' &&
    value.forEachId.length > 0 &&
    ['running', 'succeeded', 'failed', 'cancelled'].includes(
      String(value.status)
    ) &&
    ['total', 'succeeded', 'failed', 'skipped', 'active', 'pending'].every(
      key =>
        Number.isSafeInteger(value[key]) &&
        Number(value[key]) >= 0 &&
        Number(value[key]) <= 10_000
    ) &&
    Number(value.succeeded) +
      Number(value.failed) +
      Number(value.skipped) +
      Number(value.active) +
      Number(value.pending) ===
      Number(value.total) &&
    Number.isSafeInteger(value.concurrency) &&
    Number(value.concurrency) >= 1 &&
    Number(value.concurrency) <= 64
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

function dispatchExecutionOrOutputProgress(
  message: string,
  options: Pick<RustExecutorOptions, 'onProgress' | 'onOperationalOutput'>
): void {
  const value: unknown = JSON.parse(message);
  if (
    record(value) &&
    value.profile === 'woml.operational-output-wakeup/v1'
  ) {
    options.onOperationalOutput?.(parseOperationalOutputWakeup(message));
    return;
  }
  options.onProgress?.(parseExecutionProgress(message));
}

function wantsNativeProgress(
  options: Pick<RustExecutorOptions, 'onProgress' | 'onOperationalOutput'>
): boolean {
  return options.onProgress !== undefined || options.onOperationalOutput !== undefined;
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
      Number(event.eventSchemaVersion) <= 17 &&
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
  const common =
    /^[a-z][A-Za-z0-9]*$/.test(String(value.approvalId)) &&
    /^aprreq_[A-Za-z0-9_-]+$/.test(String(value.requestId)) &&
    (value.name === undefined ||
      (typeof value.name === 'string' && value.name.length > 0)) &&
    (value.description === undefined ||
      (typeof value.description === 'string' &&
        value.description.length > 0)) &&
    (value.expiresAt === undefined || dateTime(value.expiresAt)) &&
    (value.onTimeout === 'reject' || value.onTimeout === 'fail');
  if (!common) return false;
  if ('token' in value || 'credentialExpiresAt' in value) {
    return (
      exactKeys(
        value,
        [
          'approvalId',
          'requestId',
          'onTimeout',
          'token',
          'credentialExpiresAt',
        ],
        ['name', 'description', 'expiresAt']
      ) &&
      /^apr_[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$/.test(String(value.token)) &&
      dateTime(value.credentialExpiresAt)
    );
  }
  return (
    exactKeys(
      value,
      [
        'approvalId',
        'requestId',
        'onTimeout',
        'reviewDigest',
        'review',
        'authority',
      ],
      ['name', 'description', 'expiresAt']
    ) &&
    /^sha256:[a-f0-9]{64}$/.test(String(value.reviewDigest)) &&
    record(value.review) &&
    record(value.authority) &&
    exactKeys(value.authority, [
      'required',
      'received',
      'requiredRoles',
      'representedRoles',
    ]) &&
    Number.isSafeInteger(value.authority.required) &&
    Number(value.authority.required) >= 1 &&
    Number(value.authority.required) <= 20 &&
    Number.isSafeInteger(value.authority.received) &&
    Number(value.authority.received) >= 0 &&
    Array.isArray(value.authority.requiredRoles) &&
    value.authority.requiredRoles.every(role => typeof role === 'string') &&
    Array.isArray(value.authority.representedRoles) &&
    value.authority.representedRoles.every(role => typeof role === 'string')
  );
}

function waitingTimer(value: unknown): value is WaitingTimer {
  if (!record(value)) return false;
  return (
    exactKeys(value, [
      'waitId',
      'nodeId',
      'kind',
      'registeredAt',
      'deadline',
    ]) &&
    /^[a-z][A-Za-z0-9]*$/.test(String(value.waitId)) &&
    /^[A-Za-z_][A-Za-z0-9_.:-]*$/.test(String(value.nodeId)) &&
    (value.kind === 'duration' || value.kind === 'until') &&
    dateTime(value.registeredAt) &&
    dateTime(value.deadline)
  );
}

function waitingSignal(value: unknown): value is WaitingSignal {
  if (!record(value)) return false;
  return (
    exactKeys(value, [
      'waitId',
      'nodeId',
      'registeredAt',
      'signals',
      ...(value.deadline === undefined ? [] : ['deadline']),
    ]) &&
    /^[a-z][A-Za-z0-9]*$/.test(String(value.waitId)) &&
    /^[A-Za-z_][A-Za-z0-9_.:-]*$/.test(String(value.nodeId)) &&
    dateTime(value.registeredAt) &&
    Array.isArray(value.signals) &&
    value.signals.length > 0 &&
    value.signals.every(
      signal => typeof signal === 'string' && signal.length > 0
    ) &&
    (value.deadline === undefined || dateTime(value.deadline))
  );
}

function parseApprovalRuntimeOutcome(json: string): RustApprovalRuntimeOutcome {
  const value: unknown = JSON.parse(json);
  if (
    !record(value) ||
    value.contract !== 'woml.runtime-outcome' ||
    value.version !== 1
  ) {
    throw new Error('The native core returned an invalid runtime outcome.');
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
  if (
    value.status === 'waiting_time' &&
    exactKeys(value, [
      'contract',
      'version',
      'status',
      'workflowId',
      'runId',
      'wait',
    ]) &&
    typeof value.workflowId === 'string' &&
    value.workflowId.length > 0 &&
    typeof value.runId === 'string' &&
    value.runId.length > 0 &&
    waitingTimer(value.wait)
  ) {
    return value as unknown as RustApprovalRuntimeOutcome;
  }
  if (
    value.status === 'waiting_signal' &&
    exactKeys(value, [
      'contract',
      'version',
      'status',
      'workflowId',
      'runId',
      'wait',
    ]) &&
    typeof value.workflowId === 'string' &&
    value.workflowId.length > 0 &&
    typeof value.runId === 'string' &&
    value.runId.length > 0 &&
    waitingSignal(value.wait)
  ) {
    return value as unknown as RustApprovalRuntimeOutcome;
  }
  throw new Error('The native core returned an invalid runtime outcome.');
}

export function settleTimerWaitWithRust(
  eventStorePath: string,
  runId: string,
  waitId: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): TimerWaitSettlement {
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.settleWomlTimerWait !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose settleWomlTimerWait; rebuild the Rust addon.`
    );
  }
  const value: unknown = JSON.parse(
    native.settleWomlTimerWait(eventStorePath, runId, waitId)
  );
  if (
    !record(value) ||
    (value.status !== 'resolved' &&
      value.status !== 'already_resolved' &&
      value.status !== 'not_due') ||
    (value.status === 'not_due'
      ? !exactKeys(value, ['status', 'deadline']) || !dateTime(value.deadline)
      : !exactKeys(value, ['status']))
  ) {
    throw new Error('The native core returned an invalid timer settlement.');
  }
  return value as TimerWaitSettlement;
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
          decoded.code === 'WOML_APPROVAL_BINDING_MISMATCH' ||
          decoded.code === 'WOML_APPROVAL_REVIEWER_UNKNOWN' ||
          decoded.code === 'WOML_APPROVAL_REVIEWER_FORBIDDEN' ||
          decoded.code === 'WOML_APPROVAL_DUPLICATE_REVIEWER' ||
          decoded.code === 'WOML_APPROVAL_AUTHORITY_UNAVAILABLE' ||
          decoded.code === 'WOML_APPROVAL_ALREADY_RESOLVED' ||
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
      JSON.stringify(options.runtimeModules ?? []),
      options.runtimeCapabilities === undefined
        ? undefined
        : JSON.stringify(options.runtimeCapabilities)
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
    dispatchExecutionOrOutputProgress(message, options);
  };
  const execute =
    !wantsNativeProgress(options)
      ? native.executeWomlWorkflowDurable
      : native.executeWomlWorkflowDurableWithProgress;
  if (typeof execute !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose executeWomlWorkflowDurableWithProgress; rebuild the Rust addon.`
    );
  }
  const request = profileSync(
    'napi',
    'napi.serialize_durable_request',
    measurements => {
      const modelJson = JSON.stringify(workflow);
      const triggerJson = JSON.stringify(options.trigger ?? {});
      const secretsJson = JSON.stringify(options.resolvedSecrets ?? {});
      const runtimeModulesJson = JSON.stringify(options.runtimeModules ?? []);
      if (measurements !== undefined) {
        measurements.bytes.model = Buffer.byteLength(modelJson);
        measurements.bytes.trigger = Buffer.byteLength(triggerJson);
        measurements.bytes.modules = Buffer.byteLength(runtimeModulesJson);
        measurements.counts.nodes = workflow.graph.nodes.length;
        measurements.counts.modules = options.runtimeModules?.length ?? 0;
      }
      return {
        arguments: [
          modelJson,
          compiledDefinitionHash(workflow),
          triggerJson,
          options.bunExecutable ?? process.execPath,
          options.scriptHostPath ?? defaultScriptHostPath(),
          timeoutMs,
          eventStorePath,
        ] as const,
        secretsJson,
        runtimeModulesJson,
      };
    }
  );
  const resultJson = await profileAsync(
    'napi',
    'napi.execute_durable',
    async measurements => {
      const result = await (
        !wantsNativeProgress(options)
          ? native.executeWomlWorkflowDurable(
              ...request.arguments,
              request.secretsJson,
              request.runtimeModulesJson,
              options.runtimeCapabilities === undefined
                ? undefined
                : JSON.stringify(options.runtimeCapabilities)
            )
          : native.executeWomlWorkflowDurableWithProgress(
              ...request.arguments,
              progressCallback,
              request.secretsJson,
              request.runtimeModulesJson,
              options.runtimeCapabilities === undefined
                ? undefined
                : JSON.stringify(options.runtimeCapabilities)
            )
      ).catch(decodeNativeExecutionError);
      if (measurements !== undefined) {
        measurements.bytes.result = Buffer.byteLength(result);
        const decoded: unknown = JSON.parse(result);
        if (record(decoded) && typeof decoded.runId === 'string') {
          measurements.identity.runId = decoded.runId;
        }
      }
      return result;
    }
  );
  return profileSync('napi', 'napi.decode_durable_result', measurements => {
    const result = JSON.parse(resultJson) as RustWorkflowExecutionResult;
    if (measurements !== undefined) {
      measurements.bytes.result = Buffer.byteLength(resultJson);
      measurements.identity.runId = result.runId;
      measurements.counts.events = result.events.length;
    }
    return result;
  });
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
      message => dispatchExecutionOrOutputProgress(message, options),
      JSON.stringify(options.resolvedSecrets ?? {}),
      JSON.stringify(options.runtimeModules ?? []),
      options.runtimeCapabilities === undefined
        ? undefined
        : JSON.stringify(options.runtimeCapabilities)
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
    !wantsNativeProgress(options)
      ? native.executeWomlWorkflowDurableOutcome(
          ...arguments_,
          JSON.stringify(options.resolvedSecrets ?? {}),
          JSON.stringify(options.runtimeModules ?? []),
          options.runtimeCapabilities === undefined
            ? undefined
            : JSON.stringify(options.runtimeCapabilities)
        )
      : native.executeWomlWorkflowDurableOutcomeWithProgress(
          ...arguments_,
          message => dispatchExecutionOrOutputProgress(message, options),
          JSON.stringify(options.resolvedSecrets ?? {}),
          JSON.stringify(options.runtimeModules ?? []),
          options.runtimeCapabilities === undefined
            ? undefined
            : JSON.stringify(options.runtimeCapabilities)
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
    !wantsNativeProgress(options)
      ? native.resumeWomlWorkflowDurableOutcome(
          ...arguments_,
          JSON.stringify(options.resolvedSecrets ?? {}),
          JSON.stringify(options.runtimeModules ?? []),
          options.runtimeCapabilities === undefined
            ? undefined
            : JSON.stringify(options.runtimeCapabilities)
        )
      : native.resumeWomlWorkflowDurableOutcomeWithProgress(
          ...arguments_,
          message => dispatchExecutionOrOutputProgress(message, options),
          JSON.stringify(options.resolvedSecrets ?? {}),
          JSON.stringify(options.runtimeModules ?? []),
          options.runtimeCapabilities === undefined
            ? undefined
            : JSON.stringify(options.runtimeCapabilities)
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

export function resolveRichApprovalWithRust(
  eventStorePath: string,
  request: {
    readonly runId: string;
    readonly approvalId: string;
    readonly requestId: string;
    readonly reviewDigest: string;
    readonly decision: ApprovalDecision;
    readonly provider: 'slack' | 'telegram' | 'discord' | 'trusted-http';
    readonly providerAccountId: string;
    readonly providerActorId: string;
  },
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RichApprovalDecisionResult {
  const path = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(path);
  if (typeof native.resolveWomlRichApproval !== 'function') {
    throw new Error(
      `Native core at "${path}" does not expose resolveWomlRichApproval; rebuild the Rust addon.`
    );
  }
  let value: unknown;
  try {
    value = JSON.parse(
      native.resolveWomlRichApproval(
        eventStorePath,
        request.runId,
        request.approvalId,
        request.requestId,
        request.reviewDigest,
        request.decision,
        request.provider,
        request.providerAccountId,
        request.providerActorId
      )
    );
  } catch (error) {
    if (error instanceof ApprovalDecisionError) throw error;
    decodeNativeApprovalError(error);
  }
  if (
    !record(value) ||
    !exactKeys(value, [
      'contract',
      'version',
      'status',
      'runId',
      'approvalId',
      'requestId',
      'reviewDigest',
    ]) ||
    value.contract !== 'woml.approval-http' ||
    value.version !== 2 ||
    !['recorded', 'resolved', 'already_recorded', 'already_resolved'].includes(
      String(value.status)
    ) ||
    typeof value.runId !== 'string' ||
    typeof value.approvalId !== 'string' ||
    typeof value.requestId !== 'string' ||
    !/^sha256:[a-f0-9]{64}$/.test(String(value.reviewDigest))
  ) {
    throw new Error(
      'The native core returned an invalid rich approval decision.'
    );
  }
  return value as unknown as RichApprovalDecisionResult;
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
  const serialized = profileSync(
    'napi',
    'napi.serialize_runtime_registration',
    measurements => {
      const registrationsJson = JSON.stringify(registrations);
      const manualTriggersJson = JSON.stringify(
        options.startupManualTriggers ?? {}
      );
      if (measurements !== undefined) {
        measurements.bytes.registrations = Buffer.byteLength(registrationsJson);
        measurements.bytes.manual_triggers =
          Buffer.byteLength(manualTriggersJson);
        measurements.counts.workflows = registrations.length;
        measurements.counts.nodes = registrations.reduce(
          (total, registration) =>
            total + registration.workflow.graph.nodes.length,
          0
        );
      }
      return { registrationsJson, manualTriggersJson };
    }
  );
  const resultJson = await profileAsync(
    'napi',
    'napi.start_trigger_runtime',
    async measurements => {
      const result = await native
        .startWomlWebhookRuntime(
          serialized.registrationsJson,
          serialized.manualTriggersJson,
          `${options.host ?? '127.0.0.1'}:${port}`,
          eventStorePath,
          options.bunExecutable ?? process.execPath,
          options.scriptHostPath ?? defaultScriptHostPath(),
          timeoutMs,
          shutdownTimeoutMs,
          deploymentId,
          activationId,
          options.startSuspended ?? false,
          options.runtimeCapabilities === undefined
            ? undefined
            : JSON.stringify(options.runtimeCapabilities),
          options.signalIngressToken,
          options.telemetryEnabled ?? false,
          options.telemetryPropagation ?? false,
          options.telemetrySamplingRatio ?? 0,
          message => {
            const decoded: unknown = JSON.parse(message);
            if (
              record(decoded) &&
              decoded.profile === 'woml.operational-output-wakeup/v1'
            ) {
              options.onOperationalOutput?.(
                parseOperationalOutputWakeup(message)
              );
              return;
            }
            if (
              record(decoded) &&
              decoded.profile === 'woml.telemetry-signal/v1'
            ) {
              try {
                const signal = options.decodeTelemetrySignal?.(message);
                if (signal !== undefined) options.onTelemetrySignal?.(signal);
              } catch {
                // Telemetry is an optional, fail-isolated sink. A malformed
                // callback record is rejected without affecting execution.
              }
              return;
            }
            if (
              record(decoded) &&
              decoded.profile === 'woml.telemetry-admission-link/v1'
            ) {
              try {
                const link = options.decodeTelemetryAdmissionLink?.(message);
                if (link !== undefined) options.onTelemetryAdmissionLink?.(link);
              } catch {
                // Invalid optional context never affects workflow execution.
              }
              return;
            }
            if (
              record(decoded) &&
              decoded.contract === 'woml.schedule-progress'
            ) {
              options.onScheduleProgress?.(parseScheduleProgress(message));
              return;
            }
            if (
              record(decoded) &&
              decoded.contract === 'woml.interval-progress'
            ) {
              options.onIntervalProgress?.(parseIntervalProgress(message));
              return;
            }
            if (
              record(decoded) &&
              decoded.contract === 'woml.workflow-call-progress'
            ) {
              options.onWorkflowCallProgress?.(
                parseWorkflowCallProgress(message)
              );
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
              options.onRuntimeLifecycle?.(
                decoded as unknown as RuntimeInstanceV1
              );
              return;
            }
            options.onTriggerProgress?.(parseTriggerProgress(message));
          }
        )
        .catch(decodeTriggerRuntimeError);
      if (measurements !== undefined)
        measurements.bytes.result = Buffer.byteLength(result);
      return result;
    }
  );
  const value: unknown = profileSync(
    'napi',
    'napi.decode_runtime_start',
    measurements => {
      if (measurements !== undefined)
        measurements.bytes.result = Buffer.byteLength(resultJson);
      return JSON.parse(resultJson);
    }
  );
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

export async function submitExternalSignalWithRust(
  runtimeId: string,
  request: ExternalSignalAdmissionRequestV1,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): Promise<ExternalSignalOutcomeV1> {
  if (runtimeId.length === 0) throw new Error('runtimeId must not be empty.');
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.submitWomlExternalSignal !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose external signal admission; rebuild the Rust addon.`
    );
  }
  const resultJson = await native
    .submitWomlExternalSignal(runtimeId, JSON.stringify(request))
    .catch(decodeTriggerRuntimeError);
  const value: unknown = JSON.parse(resultJson);
  if (
    !record(value) ||
    value.profile !== 'woml.signal-result/v1' ||
    ![
      'accepted',
      'duplicate',
      'identity_conflict',
      'already_resolved',
      'target_not_found',
      'rejected',
    ].includes(String(value.status)) ||
    typeof value.signalId !== 'string' ||
    value.signalId.length === 0 ||
    typeof value.runId !== 'string' ||
    value.runId.length === 0 ||
    typeof value.waitId !== 'string' ||
    value.waitId.length === 0 ||
    (value.resolution !== undefined &&
      value.resolution !== 'early' &&
      value.resolution !== 'resolved') ||
    (value.code !== undefined && typeof value.code !== 'string')
  ) {
    throw new Error(
      'The native core returned an invalid Signal Result v1 value.'
    );
  }
  return value as unknown as ExternalSignalOutcomeV1;
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
  await profileAsync('napi', 'napi.activate_trigger_runtime', async () => {
    await native
      .activateWomlWebhookRuntime(runtimeId)
      .catch(decodeTriggerRuntimeError);
  });
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
  const requestJson = profileSync(
    'napi',
    'napi.serialize_manual_trigger',
    measurements => {
      const json = JSON.stringify(request);
      if (measurements !== undefined) {
        measurements.bytes.request = Buffer.byteLength(json);
        measurements.identity.invocationId = request.requestId;
      }
      return json;
    }
  );
  const resultJson = await profileAsync(
    'napi',
    'napi.submit_manual_trigger',
    async measurements => {
      if (measurements !== undefined)
        measurements.identity.invocationId = request.requestId;
      const result = await native
        .submitWomlManualTrigger(runtimeId, requestJson)
        .catch(decodeTriggerRuntimeError);
      if (measurements !== undefined)
        measurements.bytes.result = Buffer.byteLength(result);
      return result;
    }
  );
  const value: unknown = profileSync(
    'napi',
    'napi.decode_manual_trigger',
    measurements => {
      const decoded: unknown = JSON.parse(resultJson);
      if (measurements !== undefined) {
        measurements.bytes.result = Buffer.byteLength(resultJson);
        measurements.identity.invocationId = request.requestId;
        if (record(decoded) && typeof decoded.runId === 'string') {
          measurements.identity.runId = decoded.runId;
        }
      }
      return decoded;
    }
  );
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
    exactKeys(value, ['profile', 'type', 'requestId', 'code', 'message']) &&
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

export interface RustRemoteDispatchInspectionV1 {
  readonly profile: 'woml.remote-dispatch-inspection/v1';
  readonly dispatchId: string;
  readonly runId: string;
  readonly nodeId: string;
  readonly attempt: number;
  readonly target: string;
  readonly workerId?: string;
  readonly state: string;
  readonly ambiguous: boolean;
  readonly lastHeartbeatAt?: string;
  readonly outputSequence: number;
  readonly failureCode?: string;
}

function remoteDispatchNativeError(error: unknown): RunManagementError {
  const message = error instanceof Error ? error.message : String(error);
  const match = message.match(/(WOML_[A-Z0-9_]+):/u);
  return new RunManagementError(
    match?.[1] ?? 'WOML_TARGET_UNAVAILABLE',
    'The durable remote dispatch operation failed closed.'
  );
}

export function inspectRemoteDispatchWithRust(
  eventStorePath: string,
  dispatchId: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RustRemoteDispatchInspectionV1 {
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.inspectWomlRemoteDispatch !== 'function') {
    throw new Error(`Native core at "${nativePath}" does not expose remote dispatch inspection; rebuild the Rust addon.`);
  }
  try {
    const value = JSON.parse(native.inspectWomlRemoteDispatch(eventStorePath, dispatchId)) as unknown;
    if (!record(value) || value.profile !== 'woml.remote-dispatch-inspection/v1' || value.dispatchId !== dispatchId || typeof value.state !== 'string' || typeof value.ambiguous !== 'boolean') {
      throw new Error('invalid native result');
    }
    return value as unknown as RustRemoteDispatchInspectionV1;
  } catch (error) {
    if (error instanceof SyntaxError) throw new RunManagementError('WOML_REMOTE_PROTOCOL_INVALID', 'The native core returned invalid remote dispatch data.');
    throw remoteDispatchNativeError(error);
  }
}

export function reconcileRemoteDispatchWithRust(
  eventStorePath: string,
  dispatchId: string,
  resolution: 'record-failure' | 'authorize-possible-duplicate-retry',
  actor: string,
  evidenceDigest: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): Record<string, unknown> {
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.reconcileWomlRemoteDispatch !== 'function') {
    throw new Error(`Native core at "${nativePath}" does not expose remote dispatch reconciliation; rebuild the Rust addon.`);
  }
  try {
    const value = JSON.parse(native.reconcileWomlRemoteDispatch(eventStorePath, dispatchId, resolution, actor, evidenceDigest)) as unknown;
    if (!record(value) || value.dispatchId !== dispatchId || typeof value.state !== 'string') throw new Error('invalid native result');
    return value;
  } catch (error) {
    if (error instanceof SyntaxError) throw new RunManagementError('WOML_REMOTE_PROTOCOL_INVALID', 'The native core returned invalid remote dispatch data.');
    throw remoteDispatchNativeError(error);
  }
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
  const value = profileSync(
    'napi',
    'napi.inspect_run_presentation',
    measurements => {
      if (measurements !== undefined) measurements.identity.runId = runId;
      const inspected = callRunManagementNative(() =>
        native.inspectWomlRunPresentation(eventStorePath, runId)
      );
      return inspected;
    }
  );
  return profileSync(
    'presentation',
    'presentation.decode_run_presentation',
    measurements => {
      if (measurements !== undefined) measurements.identity.runId = runId;
      const encoded = JSON.stringify(value);
      if (measurements !== undefined)
        measurements.bytes.presentation = Buffer.byteLength(encoded);
      return decodeRunPresentationV1(encoded);
    }
  );
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
  if (
    eventStorePath.length === 0 ||
    workflowId.length === 0 ||
    workflowId.length > 256
  ) {
    throw new Error(
      'Workflow definition lookup requires a store path and workflow ID.'
    );
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
    candidate.every(
      item => typeof item === 'string' && item.length > 0 && item.length <= 320
    );
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
    throw new Error(
      'The native core returned invalid runtime-observation data.'
    );
  }
  return value as unknown as RustRuntimeObservationV1;
}

function callOperationalOutputNative<T>(
  call: () => string,
  decode: (json: string) => T
): T {
  try {
    return decode(call());
  } catch (error) {
    if (error instanceof OperationalOutputError) throw error;
    const message = error instanceof Error ? error.message : String(error);
    const code = message.match(/\b(WOML_OUTPUT_[A-Z0-9_]+)\b/)?.[1];
    throw new OperationalOutputError(
      code ?? 'WOML_OUTPUT_STORE_UNAVAILABLE',
      code === undefined
        ? 'The operational-output store is unavailable.'
        : message,
      { cause: error }
    );
  }
}

export function readOperationalOutputPageWithRust(
  eventStorePath: string,
  request: {
    readonly runId?: string;
    readonly after?: number;
    readonly limit?: number;
  } = {},
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): OperationalPageV1 {
  const after = request.after ?? 0;
  const limit = request.limit ?? 200;
  if (
    eventStorePath.length === 0 ||
    (request.runId !== undefined && request.runId.length === 0) ||
    !Number.isSafeInteger(after) ||
    after < 0 ||
    !Number.isSafeInteger(limit) ||
    limit < 1 ||
    limit > 1_000
  ) {
    throw new OperationalOutputError(
      'WOML_OUTPUT_CURSOR_INVALID',
      'Operational-output replay requires a valid store, cursor, and limit.'
    );
  }
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.readWomlOperationalOutputPage !== 'function') {
    throw new OperationalOutputError(
      'WOML_OUTPUT_API_UNAVAILABLE',
      `Native core at "${nativePath}" does not expose operational-output replay; rebuild the Rust addon.`
    );
  }
  return callOperationalOutputNative(
    () =>
      native.readWomlOperationalOutputPage!(
        eventStorePath,
        request.runId,
        after,
        limit
      ),
    decodeOperationalPageV1
  );
}

export function inspectOperationalProgressWithRust(
  eventStorePath: string,
  runId: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): readonly OperationalProgressSnapshotV1[] {
  if (eventStorePath.length === 0 || runId.length === 0) {
    throw new OperationalOutputError(
      'WOML_OUTPUT_SUBJECT_INVALID',
      'Operational progress requires a store path and run ID.'
    );
  }
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.inspectWomlOperationalProgress !== 'function') {
    throw new OperationalOutputError(
      'WOML_OUTPUT_API_UNAVAILABLE',
      `Native core at "${nativePath}" does not expose operational progress; rebuild the Rust addon.`
    );
  }
  return callOperationalOutputNative(
    () => native.inspectWomlOperationalProgress!(eventStorePath, runId),
    decodeOperationalProgressV1
  );
}

export function observeOperationalOutputWithRust(
  eventStorePath: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): OperationalOutputObservationV1 {
  if (eventStorePath.length === 0) {
    throw new OperationalOutputError(
      'WOML_OUTPUT_STORE_UNAVAILABLE',
      'Operational-output observation requires a store path.'
    );
  }
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.observeWomlOperationalOutput !== 'function') {
    throw new OperationalOutputError(
      'WOML_OUTPUT_API_UNAVAILABLE',
      `Native core at "${nativePath}" does not expose operational-output observation; rebuild the Rust addon.`
    );
  }
  return callOperationalOutputNative(
    () => native.observeWomlOperationalOutput!(eventStorePath),
    decodeOperationalObservationV1
  );
}

export interface OperationalOutputPrunePlanV1 {
  readonly profile: 'woml.operational-output-prune-plan/v1';
  readonly eligibleRuns: number;
  readonly eligibleRecords: number;
  readonly eligibleOutputBytes: number;
}

export interface OperationalOutputPruneResultV1 {
  readonly profile: 'woml.operational-output-prune/v1';
  readonly prunedRuns: number;
  readonly prunedRecords: number;
  readonly prunedOutputBytes: number;
  readonly prunedThrough: number;
}

function decodeOutputRetention<T>(
  json: string,
  profile: string,
  fields: readonly string[]
): T {
  const value: unknown = JSON.parse(json);
  if (
    value === null ||
    typeof value !== 'object' ||
    Array.isArray(value) ||
    (value as Record<string, unknown>).profile !== profile ||
    !fields.every(field => {
      const candidate = (value as Record<string, unknown>)[field];
      return Number.isSafeInteger(candidate) && Number(candidate) >= 0;
    })
  ) {
    throw new OperationalOutputError(
      'WOML_OUTPUT_PROTOCOL_INVALID',
      'The native core returned invalid operational-output retention data.'
    );
  }
  return value as T;
}

export function planOperationalOutputPruneWithRust(
  eventStorePath: string,
  cutoff: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): OperationalOutputPrunePlanV1 {
  const native = loadNativeCore(options.nativeCorePath ?? defaultNativeCorePath());
  if (typeof native.planWomlOperationalOutputPrune !== 'function') {
    throw new OperationalOutputError(
      'WOML_OUTPUT_API_UNAVAILABLE',
      'The native core does not expose operational-output retention planning.'
    );
  }
  return callOperationalOutputNative(
    () => native.planWomlOperationalOutputPrune!(eventStorePath, cutoff),
    json => decodeOutputRetention<OperationalOutputPrunePlanV1>(
      json,
      'woml.operational-output-prune-plan/v1',
      ['eligibleRuns', 'eligibleRecords', 'eligibleOutputBytes']
    )
  );
}

export function pruneOperationalOutputWithRust(
  eventStorePath: string,
  cutoff: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): OperationalOutputPruneResultV1 {
  const native = loadNativeCore(options.nativeCorePath ?? defaultNativeCorePath());
  if (typeof native.pruneWomlOperationalOutput !== 'function') {
    throw new OperationalOutputError(
      'WOML_OUTPUT_API_UNAVAILABLE',
      'The native core does not expose operational-output pruning.'
    );
  }
  return callOperationalOutputNative(
    () => native.pruneWomlOperationalOutput!(eventStorePath, cutoff),
    json => decodeOutputRetention<OperationalOutputPruneResultV1>(
      json,
      'woml.operational-output-prune/v1',
      ['prunedRuns', 'prunedRecords', 'prunedOutputBytes', 'prunedThrough']
    )
  );
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
      [
        'deploymentId',
        'activationId',
        'runtimeInstanceId',
        'runtimeLeaseExpiresAt',
      ]
    ) ||
    ![13, 14, 15, 16, 17, 18, 19, 20].includes(
      Number(value.storeVersion)
    ) ||
    !Array.isArray(value.definitionHashes) ||
    value.definitionHashes.length < 1 ||
    value.definitionHashes.length > 10_000 ||
    !value.definitionHashes.every(digest) ||
    new Set(value.definitionHashes).size !== value.definitionHashes.length ||
    (value.deploymentId !== undefined &&
      (typeof value.deploymentId !== 'string' ||
        value.deploymentId.length === 0)) ||
    (value.activationId !== undefined && !digest(value.activationId)) ||
    (value.runtimeInstanceId !== undefined &&
      (typeof value.runtimeInstanceId !== 'string' ||
        value.runtimeInstanceId.length === 0)) ||
    (value.runtimeLeaseExpiresAt !== undefined &&
      !dateTime(value.runtimeLeaseExpiresAt))
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
  const native = loadNativeCore(
    options.nativeCorePath ?? defaultNativeCorePath()
  );
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
  const native = loadNativeCore(
    options.nativeCorePath ?? defaultNativeCorePath()
  );
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
  const native = loadNativeCore(
    options.nativeCorePath ?? defaultNativeCorePath()
  );
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
  const native = loadNativeCore(
    options.nativeCorePath ?? defaultNativeCorePath()
  );
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
  const native = loadNativeCore(
    options.nativeCorePath ?? defaultNativeCorePath()
  );
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
  const native = loadNativeCore(
    options.nativeCorePath ?? defaultNativeCorePath()
  );
  if (typeof native.executeWomlRetention !== 'function') {
    throw new RetentionOperationError(
      'WOML_RETENTION_UNAVAILABLE',
      'The native core does not expose retention execution; rebuild the Rust addon.'
    );
  }
  try {
    return decodeRetentionExecution(
      JSON.parse(
        native.executeWomlRetention(
          eventStorePath,
          JSON.stringify(policy),
          leaseId,
          ownerId,
          compact,
          now
        )
      )
    );
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
  const native = loadNativeCore(
    options.nativeCorePath ?? defaultNativeCorePath()
  );
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
  const native = loadNativeCore(
    options.nativeCorePath ?? defaultNativeCorePath()
  );
  if (typeof native.readWomlLastRetentionResult !== 'function')
    return undefined;
  try {
    const value: unknown = JSON.parse(
      native.readWomlLastRetentionResult(eventStorePath)
    );
    return value === null ? undefined : retentionResult(value);
  } catch (error) {
    if (error instanceof RetentionOperationError) throw error;
    decodeRetentionNativeError(error);
  }
}

export function maintainArtifactsWithRust(
  eventStorePath: string,
  authority: ArtifactRuntimeAuthorityV1,
  resolvedSecrets: Readonly<Record<string, string>>,
  now: string,
  dryRun: boolean,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RustArtifactMaintenanceV1 {
  const native = loadNativeCore(
    options.nativeCorePath ?? defaultNativeCorePath()
  );
  if (typeof native.maintainWomlArtifacts !== 'function') {
    throw new RetentionOperationError(
      'WOML_ARTIFACT_MAINTENANCE_UNAVAILABLE',
      'The native core does not expose artifact maintenance; rebuild the Rust addon.'
    );
  }
  let value: unknown;
  try {
    value = JSON.parse(
      native.maintainWomlArtifacts(
        eventStorePath,
        JSON.stringify(authority.artifacts.policy),
        JSON.stringify(authority.artifacts.mode),
        JSON.stringify(resolvedSecrets),
        now,
        dryRun
      )
    );
  } catch (error) {
    throw new RetentionOperationError(
      'WOML_ARTIFACT_MAINTENANCE_FAILED',
      error instanceof Error ? error.message : 'Artifact maintenance failed.'
    );
  }
  const fields = [
    'profile',
    'kind',
    'completedAt',
    'eligibleArtifacts',
    'eligibleBlobs',
    'estimatedBytes',
    'deletedArtifacts',
    'deletedBlobs',
    'deletedBytes',
    'scrubbedBlobs',
    'corruptBlobs',
    'collectedOrphans',
    'liveArtifacts',
    'liveBlobs',
    'liveBytes',
  ];
  if (
    !record(value) ||
    !exactKeys(value, fields) ||
    value.profile !== 'woml.artifact-maintenance/v1' ||
    !['plan', 'result'].includes(String(value.kind)) ||
    !dateTime(value.completedAt) ||
    fields.slice(3).some(field => !count(value[field]))
  ) {
    throw new RetentionOperationError(
      'WOML_ARTIFACT_MAINTENANCE_RESULT_INVALID',
      'The native core returned invalid Artifact Maintenance v1 data.'
    );
  }
  return value as unknown as RustArtifactMaintenanceV1;
}

function artifactOperationFailure(error: unknown, fallbackCode: string): never {
  if (error instanceof ArtifactOperationError) throw error;
  const message =
    error instanceof Error ? error.message : 'Artifact operation failed.';
  const code = message.match(/\b(WOML_[A-Z0-9_]+):/)?.[1] ?? fallbackCode;
  throw new ArtifactOperationError(code, message);
}

function artifactInspection(value: unknown): RustArtifactInspectionV1 {
  if (
    !record(value) ||
    !exactKeys(value, [
      'profile',
      'artifact',
      'digestPrefix',
      'status',
      'createdAt',
      'expiresAt',
      'retentionMs',
      'provenance',
    ]) ||
    value.profile !== 'woml.artifact-inspection/v1' ||
    !record(value.artifact) ||
    !exactKeys(value.artifact, [
      'contract',
      'contractVersion',
      'id',
      'name',
      'size',
      'sha256',
      'contentType',
    ]) ||
    value.artifact.contract !== 'woml.artifact-reference' ||
    value.artifact.contractVersion !== 1 ||
    typeof value.artifact.id !== 'string' ||
    !/^artifact_[0-9a-hjkmnp-tv-z]{26}$/.test(value.artifact.id) ||
    typeof value.artifact.name !== 'string' ||
    !count(value.artifact.size) ||
    typeof value.artifact.sha256 !== 'string' ||
    !/^[0-9a-f]{64}$/.test(value.artifact.sha256) ||
    typeof value.artifact.contentType !== 'string' ||
    typeof value.digestPrefix !== 'string' ||
    !/^[0-9a-f]{12}$/.test(value.digestPrefix) ||
    !['available', 'quarantined', 'expired', 'corrupt', 'deleted'].includes(
      String(value.status)
    ) ||
    !dateTime(value.createdAt) ||
    !dateTime(value.expiresAt) ||
    !count(value.retentionMs) ||
    !record(value.provenance) ||
    !exactKeys(value.provenance, [
      'workflowId',
      'runId',
      'nodeId',
      'attempt',
    ]) ||
    typeof value.provenance.workflowId !== 'string' ||
    typeof value.provenance.runId !== 'string' ||
    typeof value.provenance.nodeId !== 'string' ||
    !count(value.provenance.attempt)
  ) {
    throw new ArtifactOperationError(
      'WOML_ARTIFACT_INSPECTION_INVALID',
      'The native core returned invalid Artifact Inspection v1 data.'
    );
  }
  return value as unknown as RustArtifactInspectionV1;
}

export function inspectArtifactWithRust(
  eventStorePath: string,
  artifactId: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RustArtifactInspectionV1 {
  const native = loadNativeCore(
    options.nativeCorePath ?? defaultNativeCorePath()
  );
  if (typeof native.inspectWomlArtifact !== 'function') {
    throw new ArtifactOperationError(
      'WOML_ARTIFACT_INSPECTION_UNAVAILABLE',
      'The native core does not expose artifact inspection; rebuild the Rust addon.'
    );
  }
  try {
    return artifactInspection(
      JSON.parse(native.inspectWomlArtifact(eventStorePath, artifactId))
    );
  } catch (error) {
    artifactOperationFailure(error, 'WOML_ARTIFACT_INSPECTION_FAILED');
  }
}

export function inspectRunArtifactsWithRust(
  eventStorePath: string,
  runId: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RustArtifactRunInspectionV1 {
  const native = loadNativeCore(
    options.nativeCorePath ?? defaultNativeCorePath()
  );
  if (typeof native.inspectWomlRunArtifacts !== 'function') {
    throw new ArtifactOperationError(
      'WOML_ARTIFACT_INSPECTION_UNAVAILABLE',
      'The native core does not expose run artifact inspection; rebuild the Rust addon.'
    );
  }
  try {
    const value: unknown = JSON.parse(
      native.inspectWomlRunArtifacts(eventStorePath, runId)
    );
    if (
      !record(value) ||
      !exactKeys(value, ['profile', 'runId', 'artifacts', 'truncated']) ||
      value.profile !== 'woml.artifact-run-inspection/v1' ||
      value.runId !== runId ||
      !Array.isArray(value.artifacts) ||
      value.artifacts.length > 100 ||
      typeof value.truncated !== 'boolean'
    ) {
      throw new ArtifactOperationError(
        'WOML_ARTIFACT_INSPECTION_INVALID',
        'The native core returned invalid run artifact inspection data.'
      );
    }
    return {
      profile: 'woml.artifact-run-inspection/v1',
      runId,
      artifacts: value.artifacts.map(artifactInspection),
      truncated: value.truncated,
    };
  } catch (error) {
    artifactOperationFailure(error, 'WOML_ARTIFACT_INSPECTION_FAILED');
  }
}

export function exportArtifactWithRust(
  eventStorePath: string,
  destinationPath: string,
  artifactId: string,
  authority: ArtifactRuntimeAuthorityV1,
  resolvedSecrets: Readonly<Record<string, string>>,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RustArtifactExportV1 {
  const native = loadNativeCore(
    options.nativeCorePath ?? defaultNativeCorePath()
  );
  if (typeof native.exportWomlArtifact !== 'function') {
    throw new ArtifactOperationError(
      'WOML_ARTIFACT_EXPORT_UNAVAILABLE',
      'The native core does not expose artifact export; rebuild the Rust addon.'
    );
  }
  try {
    const value: unknown = JSON.parse(
      native.exportWomlArtifact(
        eventStorePath,
        destinationPath,
        artifactId,
        JSON.stringify(authority.artifacts.policy),
        JSON.stringify(authority.artifacts.mode),
        JSON.stringify(resolvedSecrets)
      )
    );
    if (
      !record(value) ||
      !exactKeys(value, [
        'profile',
        'auditId',
        'artifactId',
        'name',
        'size',
        'digestPrefix',
        'exportedAt',
        'verified',
      ]) ||
      value.profile !== 'woml.artifact-export/v1' ||
      typeof value.auditId !== 'string' ||
      value.artifactId !== artifactId ||
      typeof value.name !== 'string' ||
      !count(value.size) ||
      typeof value.digestPrefix !== 'string' ||
      !/^[0-9a-f]{12}$/.test(value.digestPrefix) ||
      !dateTime(value.exportedAt) ||
      value.verified !== true
    ) {
      throw new ArtifactOperationError(
        'WOML_ARTIFACT_EXPORT_INVALID',
        'The native core returned invalid Artifact Export v1 data.'
      );
    }
    return value as unknown as RustArtifactExportV1;
  } catch (error) {
    artifactOperationFailure(error, 'WOML_ARTIFACT_EXPORT_FAILED');
  }
}

export function observeArtifactsWithRust(
  eventStorePath: string,
  authority: ArtifactRuntimeAuthorityV1,
  resolvedSecrets: Readonly<Record<string, string>>,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RustArtifactObservationV1 {
  const native = loadNativeCore(
    options.nativeCorePath ?? defaultNativeCorePath()
  );
  if (typeof native.observeWomlArtifacts !== 'function') {
    throw new ArtifactOperationError(
      'WOML_ARTIFACT_OBSERVATION_UNAVAILABLE',
      'The native core does not expose artifact observation; rebuild the Rust addon.'
    );
  }
  try {
    const value: unknown = JSON.parse(
      native.observeWomlArtifacts(
        eventStorePath,
        JSON.stringify(authority.artifacts.policy),
        JSON.stringify(authority.artifacts.mode),
        JSON.stringify(resolvedSecrets)
      )
    );
    const numeric = [
      'transfers',
      'transferredBytes',
      'transferDurationMs',
      'failedTransfers',
      'corruptArtifacts',
      'orphanBlobs',
      'liveArtifacts',
      'liveBytes',
    ];
    if (
      !record(value) ||
      !exactKeys(value, ['profile', 'backend', 'backendHealthy', ...numeric]) ||
      value.profile !== 'woml.artifact-observation/v1' ||
      !['local', 's3'].includes(String(value.backend)) ||
      typeof value.backendHealthy !== 'boolean' ||
      numeric.some(field => !count(value[field]))
    ) {
      throw new ArtifactOperationError(
        'WOML_ARTIFACT_OBSERVATION_INVALID',
        'The native core returned invalid Artifact Observation v1 data.'
      );
    }
    return value as unknown as RustArtifactObservationV1;
  } catch (error) {
    artifactOperationFailure(error, 'WOML_ARTIFACT_OBSERVATION_FAILED');
  }
}

function artifactBackupInventory(
  value: unknown
): RustArtifactBackupInventoryV1 {
  if (
    !record(value) ||
    !exactKeys(value, [
      'profile',
      'directory',
      'blobCount',
      'totalBytes',
      'inventoryDigest',
      'verified',
    ]) ||
    value.profile !== 'woml.artifact-backup-inventory/v1' ||
    value.directory !== 'artifacts' ||
    !count(value.blobCount) ||
    !count(value.totalBytes) ||
    typeof value.inventoryDigest !== 'string' ||
    !/^sha256:[0-9a-f]{64}$/.test(value.inventoryDigest) ||
    value.verified !== true
  ) {
    throw new BackupOperationError(
      'WOML_BACKUP_VERIFICATION_FAILED',
      'The native core returned invalid artifact backup inventory.'
    );
  }
  return value as unknown as RustArtifactBackupInventoryV1;
}

export function exportArtifactBackupWithRust(
  eventStorePath: string,
  destinationPath: string,
  authority: ArtifactRuntimeAuthorityV1,
  resolvedSecrets: Readonly<Record<string, string>>,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RustArtifactBackupInventoryV1 {
  const native = loadNativeCore(
    options.nativeCorePath ?? defaultNativeCorePath()
  );
  if (typeof native.exportWomlArtifactBackup !== 'function') {
    throw new BackupOperationError(
      'WOML_BACKUP_UNAVAILABLE',
      'The native core does not expose artifact backup; rebuild the Rust addon.'
    );
  }
  try {
    return artifactBackupInventory(
      JSON.parse(
        native.exportWomlArtifactBackup(
          eventStorePath,
          destinationPath,
          JSON.stringify(authority.artifacts.policy),
          JSON.stringify(authority.artifacts.mode),
          JSON.stringify(resolvedSecrets)
        )
      )
    );
  } catch (error) {
    if (error instanceof BackupOperationError) throw error;
    throw new BackupOperationError(
      'WOML_BACKUP_VERIFICATION_FAILED',
      error instanceof Error ? error.message : 'Artifact backup failed.'
    );
  }
}

export function restoreArtifactBackupWithRust(
  eventStorePath: string,
  sourcePath: string,
  authority: ArtifactRuntimeAuthorityV1,
  resolvedSecrets: Readonly<Record<string, string>>,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RustArtifactBackupInventoryV1 {
  const native = loadNativeCore(
    options.nativeCorePath ?? defaultNativeCorePath()
  );
  if (typeof native.restoreWomlArtifactBackup !== 'function') {
    throw new BackupOperationError(
      'WOML_RESTORE_UNAVAILABLE',
      'The native core does not expose artifact restore; rebuild the Rust addon.'
    );
  }
  try {
    return artifactBackupInventory(
      JSON.parse(
        native.restoreWomlArtifactBackup(
          eventStorePath,
          sourcePath,
          JSON.stringify(authority.artifacts.policy),
          JSON.stringify(authority.artifacts.mode),
          JSON.stringify(resolvedSecrets)
        )
      )
    );
  } catch (error) {
    if (error instanceof BackupOperationError) throw error;
    throw new BackupOperationError(
      'WOML_RESTORE_VERIFICATION_FAILED',
      error instanceof Error ? error.message : 'Artifact restore failed.'
    );
  }
}

export function verifyArtifactBackupWithRust(
  eventStorePath: string,
  sourcePath: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RustArtifactBackupInventoryV1 {
  const native = loadNativeCore(
    options.nativeCorePath ?? defaultNativeCorePath()
  );
  if (typeof native.verifyWomlArtifactBackup !== 'function') {
    throw new BackupOperationError(
      'WOML_BACKUP_UNAVAILABLE',
      'The native core does not expose artifact backup verification; rebuild the Rust addon.'
    );
  }
  try {
    return artifactBackupInventory(
      JSON.parse(native.verifyWomlArtifactBackup(eventStorePath, sourcePath))
    );
  } catch (error) {
    if (error instanceof BackupOperationError) throw error;
    throw new BackupOperationError(
      'WOML_BACKUP_VERIFICATION_FAILED',
      error instanceof Error
        ? error.message
        : 'Artifact backup verification failed.'
    );
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
      'pending',
      'running',
      'succeeded',
      'failed',
      'cancelled',
      'completedWithWarnings',
    ]) &&
    Object.values(candidate.counts).every(
      count => Number.isSafeInteger(count) && Number(count) >= 0
    ) &&
    Array.isArray(candidate.items) &&
    candidate.items.length <= 256 &&
    candidate.items.every(
      item =>
        record(item) &&
        exactKeys(item, [
          'invocationId',
          'alias',
          'definitionDigestPrefix',
          'kind',
          'status',
          'lifecycleStatus',
        ]) &&
        typeof item.invocationId === 'string' &&
        typeof item.alias === 'string' &&
        /^[a-f0-9]{12}$/.test(String(item.definitionDigestPrefix)) &&
        ['step', 'notification-provider'].includes(String(item.kind)) &&
        ['pending', 'running', 'succeeded', 'failed', 'cancelled'].includes(
          String(item.status)
        ) &&
        ['idle', 'running', 'completed', 'completed_with_warnings'].includes(
          String(item.lifecycleStatus)
        )
    );
  const forEach = (candidate: unknown): boolean => {
    if (
      !record(candidate) ||
      !exactKeys(candidate, ['counts', 'items']) ||
      !record(candidate.counts) ||
      !exactKeys(candidate.counts, [
        'opened',
        'active',
        'succeeded',
        'failed',
        'cancelled',
      ]) ||
      !Object.values(candidate.counts).every(
        count =>
          Number.isSafeInteger(count) &&
          Number(count) >= 0 &&
          Number(count) <= 10_000
      ) ||
      !Array.isArray(candidate.items) ||
      candidate.items.length > 10_000
    )
      return false;
    const validItems = candidate.items.every(item => {
      if (
        !record(item) ||
        !exactKeys(
          item,
          [
            'forEachId',
            'status',
            'total',
            'succeeded',
            'failed',
            'skipped',
            'active',
            'pending',
            'concurrency',
          ],
          ['failedIndex', 'failedNodeId']
        ) ||
        typeof item.forEachId !== 'string' ||
        !['active', 'succeeded', 'failed', 'cancelled'].includes(
          String(item.status)
        ) ||
        !['total', 'succeeded', 'failed', 'skipped', 'active', 'pending'].every(
          field =>
            Number.isSafeInteger(item[field]) &&
            Number(item[field]) >= 0 &&
            Number(item[field]) <= 10_000
        ) ||
        !Number.isSafeInteger(item.concurrency) ||
        Number(item.concurrency) < 1 ||
        Number(item.concurrency) > 64 ||
        Number(item.succeeded) +
          Number(item.failed) +
          Number(item.skipped) +
          Number(item.active) +
          Number(item.pending) !==
          Number(item.total) ||
        (item.failedIndex !== undefined &&
          (!Number.isSafeInteger(item.failedIndex) ||
            Number(item.failedIndex) < 0 ||
            Number(item.failedIndex) >= Number(item.total))) ||
        (item.failedNodeId !== undefined &&
          typeof item.failedNodeId !== 'string')
      )
        return false;
      return true;
    });
    if (
      !validItems ||
      Number(candidate.counts.opened) !== candidate.items.length
    )
      return false;
    for (const status of [
      'active',
      'succeeded',
      'failed',
      'cancelled',
    ] as const) {
      if (
        Number(candidate.counts[status]) !==
        candidate.items.filter(item => item.status === status).length
      )
        return false;
    }
    return true;
  };
  const approvals = (candidate: unknown): boolean => {
    if (
      !record(candidate) ||
      !exactKeys(candidate, ['pending', 'items', 'truncated']) ||
      !Number.isSafeInteger(candidate.pending) ||
      Number(candidate.pending) < 0 ||
      typeof candidate.truncated !== 'boolean' ||
      !Array.isArray(candidate.items) ||
      candidate.items.length > 100
    )
      return false;
    const valid = candidate.items.every(
      item =>
        record(item) &&
        exactKeys(
          item,
          [
            'approvalId',
            'requestId',
            'status',
            'reviewDigest',
            'review',
            'required',
            'received',
            'requiredRoles',
            'representedRoles',
            'directoryVersion',
            'requestedAt',
            'decisions',
            'totalDecisions',
            'decisionsTruncated',
          ],
          ['deadline', 'resolvedAt']
        ) &&
        typeof item.approvalId === 'string' &&
        typeof item.requestId === 'string' &&
        typeof item.status === 'string' &&
        /^sha256:[a-f0-9]{64}$/.test(String(item.reviewDigest)) &&
        record(item.review) &&
        Number.isSafeInteger(item.required) &&
        Number(item.required) >= 1 &&
        Number(item.required) <= 20 &&
        Number.isSafeInteger(item.received) &&
        Number(item.received) >= 0 &&
        Number(item.received) <= Number(item.required) &&
        Array.isArray(item.requiredRoles) &&
        item.requiredRoles.length <= 16 &&
        item.requiredRoles.every(role => typeof role === 'string') &&
        Array.isArray(item.representedRoles) &&
        item.representedRoles.length <= 16 &&
        item.representedRoles.every(role => typeof role === 'string') &&
        typeof item.directoryVersion === 'string' &&
        dateTime(item.requestedAt) &&
        (item.deadline === undefined || dateTime(item.deadline)) &&
        (item.resolvedAt === undefined || dateTime(item.resolvedAt)) &&
        Array.isArray(item.decisions) &&
        item.decisions.length <= 100 &&
        item.decisions.every(
          decision =>
            record(decision) &&
            exactKeys(decision, [
              'principalId',
              'decision',
              'provider',
              'providerAccountId',
              'roles',
              'decidedAt',
            ]) &&
            typeof decision.principalId === 'string' &&
            ['approved', 'rejected'].includes(String(decision.decision)) &&
            typeof decision.provider === 'string' &&
            typeof decision.providerAccountId === 'string' &&
            Array.isArray(decision.roles) &&
            decision.roles.every(role => typeof role === 'string') &&
            dateTime(decision.decidedAt)
        ) &&
        Number.isSafeInteger(item.totalDecisions) &&
        Number(item.totalDecisions) >= item.decisions.length &&
        typeof item.decisionsTruncated === 'boolean'
    );
    return (
      valid &&
      Number(candidate.pending) ===
        candidate.items.filter(
          item => record(item) && item.status === 'pending'
        ).length
    );
  };
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
      [
        'policy',
        'infrastructurePolicy',
        'forks',
        'reusableDefinitions',
        'forEach',
        'approvals',
      ]
    ) ||
    ![
      'woml.run-inspection/v2',
      'woml.run-inspection/v3',
      'woml.run-inspection/v4',
      'woml.run-inspection/v5',
      'woml.run-inspection/v6',
      'woml.run-inspection/v11',
    ].includes(String(value.profile)) ||
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
    (value.profile === 'woml.run-inspection/v2' &&
      value.policy !== undefined) ||
    (value.infrastructurePolicy !== undefined &&
      (!record(value.infrastructurePolicy) ||
        !exactKeys(value.infrastructurePolicy, [
          'profile',
          'binding',
          'decisions',
          'totalDecisions',
          'truncated',
        ]) ||
        value.infrastructurePolicy.profile !==
          'woml.infrastructure-policy-run-inspection/v1' ||
        !record(value.infrastructurePolicy.binding) ||
        !exactKeys(
          value.infrastructurePolicy.binding,
          [
            'runId',
            'policyId',
            'policyVersion',
            'policyDigest',
            'canonicalBundleReference',
            'signatureStatus',
            'admittedAt',
          ],
          ['signingKeyId']
        ) ||
        typeof value.infrastructurePolicy.binding.policyId !== 'string' ||
        typeof value.infrastructurePolicy.binding.policyVersion !== 'string' ||
        typeof value.infrastructurePolicy.binding.policyDigest !== 'string' ||
        !dateTime(value.infrastructurePolicy.binding.admittedAt) ||
        !Array.isArray(value.infrastructurePolicy.decisions) ||
        value.infrastructurePolicy.decisions.length > 100 ||
        !Number.isSafeInteger(value.infrastructurePolicy.totalDecisions) ||
        typeof value.infrastructurePolicy.truncated !== 'boolean')) ||
    ([
      'woml.run-inspection/v4',
      'woml.run-inspection/v5',
      'woml.run-inspection/v6',
      'woml.run-inspection/v11',
    ].includes(String(value.profile)) &&
      !record(value.forks)) ||
    ([
      'woml.run-inspection/v5',
      'woml.run-inspection/v6',
      'woml.run-inspection/v11',
    ].includes(String(value.profile)) &&
      !reusableDefinitions(value.reusableDefinitions)) ||
    (![
      'woml.run-inspection/v5',
      'woml.run-inspection/v6',
      'woml.run-inspection/v11',
    ].includes(String(value.profile)) &&
      value.reusableDefinitions !== undefined) ||
    (['woml.run-inspection/v6', 'woml.run-inspection/v11'].includes(
      String(value.profile)
    ) &&
      !forEach(value.forEach)) ||
    (!['woml.run-inspection/v6', 'woml.run-inspection/v11'].includes(
      String(value.profile)
    ) &&
      value.forEach !== undefined) ||
    (value.profile === 'woml.run-inspection/v11' &&
      !approvals(value.approvals)) ||
    (value.profile !== 'woml.run-inspection/v11' &&
      value.approvals !== undefined)
  ) {
    throw new Error('The native core returned invalid run-inspection data.');
  }
  return value as unknown as RustRunInspectionV2;
}

export function inspectWaitsV1WithRust(
  eventStorePath: string,
  runId: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RustWaitInspectionV1 {
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.inspectWomlWaitsV1 !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose durable-wait inspection; rebuild the Rust addon.`
    );
  }
  const value = callRunManagementNative(() =>
    native.inspectWomlWaitsV1(eventStorePath, runId)
  );
  const wait = (candidate: unknown): boolean =>
    record(candidate) &&
    exactKeys(
      candidate,
      ['runId', 'waitId', 'name', 'kind', 'status', 'registeredAt'],
      [
        'signals',
        'correlationDigest',
        'deadline',
        'resolvedAt',
        'winningSignal',
      ]
    ) &&
    candidate.runId === runId &&
    typeof candidate.waitId === 'string' &&
    typeof candidate.name === 'string' &&
    ['duration', 'until', 'signal'].includes(String(candidate.kind)) &&
    ['pending', 'signaled', 'elapsed', 'timed_out', 'cancelled'].includes(
      String(candidate.status)
    ) &&
    dateTime(candidate.registeredAt) &&
    (candidate.signals === undefined ||
      (Array.isArray(candidate.signals) &&
        candidate.signals.length <= 16 &&
        candidate.signals.every(signal => typeof signal === 'string'))) &&
    (candidate.correlationDigest === undefined ||
      /^sha256:[a-f0-9]{64}$/.test(String(candidate.correlationDigest))) &&
    (candidate.deadline === undefined || dateTime(candidate.deadline)) &&
    (candidate.resolvedAt === undefined || dateTime(candidate.resolvedAt)) &&
    (candidate.winningSignal === undefined ||
      typeof candidate.winningSignal === 'string');
  if (
    !record(value) ||
    !exactKeys(
      value,
      ['profile', 'runId', 'waits', 'pendingWaits'],
      ['oldestPendingWaitAgeMs']
    ) ||
    value.profile !== 'woml.wait-inspection/v1' ||
    value.runId !== runId ||
    !Array.isArray(value.waits) ||
    value.waits.length > 10_000 ||
    !value.waits.every(wait) ||
    !Number.isSafeInteger(value.pendingWaits) ||
    Number(value.pendingWaits) < 0 ||
    Number(value.pendingWaits) !==
      value.waits.filter(candidate => candidate.status === 'pending').length ||
    (value.oldestPendingWaitAgeMs !== undefined &&
      (!Number.isSafeInteger(value.oldestPendingWaitAgeMs) ||
        Number(value.oldestPendingWaitAgeMs) < 0))
  ) {
    throw new Error(
      'The native core returned invalid durable-wait inspection data.'
    );
  }
  return value as unknown as RustWaitInspectionV1;
}

export function inspectCorrectionV1WithRust(
  eventStorePath: string,
  runId: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RustCorrectionInspectionV1 | undefined {
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.inspectWomlCorrectionV1 !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose correction inspection; rebuild the Rust addon.`
    );
  }
  const value = callRunManagementNative(() =>
    native.inspectWomlCorrectionV1!(eventStorePath, runId)
  );
  if (value === null) return undefined;
  const count = (candidate: unknown) =>
    Number.isSafeInteger(candidate) && Number(candidate) >= 0;
  const saga = (candidate: unknown): boolean =>
    record(candidate) &&
    exactKeys(candidate, [
      'sagaId',
      'status',
      'registeredCompensations',
      'completedCompensations',
      'completedCleanup',
    ]) &&
    typeof candidate.sagaId === 'string' &&
    typeof candidate.status === 'string' &&
    count(candidate.registeredCompensations) &&
    count(candidate.completedCompensations) &&
    count(candidate.completedCleanup);
  const unresolved = (candidate: unknown): boolean =>
    record(candidate) &&
    exactKeys(candidate, ['actionId', 'kind', 'code', 'ambiguous']) &&
    typeof candidate.actionId === 'string' &&
    ['compensation', 'cleanup'].includes(String(candidate.kind)) &&
    typeof candidate.code === 'string' &&
    /^WOML_[A-Z0-9_]+$/.test(candidate.code) &&
    typeof candidate.ambiguous === 'boolean';
  if (
    !record(value) ||
    !exactKeys(value, [
      'profile',
      'runId',
      'phase',
      'originalOutcome',
      'sagas',
      'unresolvedActions',
      'truncated',
    ]) ||
    value.profile !== 'woml.correction-inspection/v1' ||
    value.runId !== runId ||
    ![
      'forward',
      'compensating',
      'cleanup',
      'completed',
      'manual_recovery',
    ].includes(String(value.phase)) ||
    !['running', 'succeeded', 'failed', 'cancelled'].includes(
      String(value.originalOutcome)
    ) ||
    !Array.isArray(value.sagas) ||
    value.sagas.length > 64 ||
    !value.sagas.every(saga) ||
    !Array.isArray(value.unresolvedActions) ||
    value.unresolvedActions.length > 100 ||
    !value.unresolvedActions.every(unresolved) ||
    typeof value.truncated !== 'boolean'
  ) {
    throw new Error(
      'The native core returned invalid Correction Inspection v1 data.'
    );
  }
  return value as unknown as RustCorrectionInspectionV1;
}

export function recoverCorrectionWithRust(
  eventStorePath: string,
  request: RustCorrectionRecoveryRequestV1,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RustCorrectionRecoveryResultV1 {
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.recoverWomlCorrection !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose correction recovery; rebuild the Rust addon.`
    );
  }
  const value = callRunManagementNative(() =>
    native.recoverWomlCorrection!(eventStorePath, JSON.stringify(request))
  );
  if (
    !record(value) ||
    !exactKeys(value, [
      'profile',
      'runId',
      'actionId',
      'status',
      'auditRecorded',
    ]) ||
    value.profile !== 'woml.correction-recovery-result/v1' ||
    value.runId !== request.runId ||
    value.actionId !== request.actionId ||
    !['retry_scheduled', 'externally_resolved', 'rejected'].includes(
      String(value.status)
    ) ||
    value.auditRecorded !== true
  ) {
    throw new Error(
      'The native core returned invalid Correction Recovery v1 data.'
    );
  }
  return value as unknown as RustCorrectionRecoveryResultV1;
}

export function observeCorrectionsV1WithRust(
  eventStorePath: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RustCorrectionObservationV1 {
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.observeWomlCorrectionsV1 !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose correction observation; rebuild the Rust addon.`
    );
  }
  const value = callRunManagementNative(() =>
    native.observeWomlCorrectionsV1!(eventStorePath)
  );
  const count = (candidate: unknown) =>
    Number.isSafeInteger(candidate) && Number(candidate) >= 0;
  if (
    !record(value) ||
    !exactKeys(
      value,
      [
        'profile',
        'openSagas',
        'pendingActions',
        'activeAttempts',
        'manualRecoveryActions',
      ],
      ['oldestManualRecoveryAgeMs']
    ) ||
    value.profile !== 'woml.correction-observation/v1' ||
    !count(value.openSagas) ||
    !count(value.pendingActions) ||
    !count(value.activeAttempts) ||
    !count(value.manualRecoveryActions) ||
    (value.oldestManualRecoveryAgeMs !== undefined &&
      !count(value.oldestManualRecoveryAgeMs))
  ) {
    throw new Error(
      'The native core returned invalid Correction Observation v1 data.'
    );
  }
  return value as unknown as RustCorrectionObservationV1;
}

export function inspectLocksV1WithRust(
  eventStorePath: string,
  runId: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): readonly RustLockInspectionV1[] {
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.inspectWomlLocksV1 !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose lock inspection; rebuild the Rust addon.`
    );
  }
  const value = callRunManagementNative(() =>
    native.inspectWomlLocksV1!(eventStorePath, runId)
  );
  const validInteger = (candidate: unknown) =>
    Number.isSafeInteger(candidate) && Number(candidate) >= 0;
  const lock = (candidate: unknown): boolean =>
    record(candidate) &&
    exactKeys(
      candidate,
      ['profile', 'runId', 'lockId', 'key', 'status', 'registeredAt'],
      [
        'acquiredAt',
        'releasedAt',
        'deadline',
        'expiresAt',
        'fencingToken',
        'waitedMs',
        'heldMs',
        'failureCode',
      ]
    ) &&
    candidate.profile === 'woml.lock-inspection/v1' &&
    candidate.runId === runId &&
    typeof candidate.lockId === 'string' &&
    candidate.lockId.length > 0 &&
    typeof candidate.key === 'string' &&
    Buffer.byteLength(candidate.key, 'utf8') <= 256 &&
    ['waiting', 'held', 'released', 'timed_out', 'cancelled', 'lost'].includes(
      String(candidate.status)
    ) &&
    dateTime(candidate.registeredAt) &&
    (candidate.acquiredAt === undefined || dateTime(candidate.acquiredAt)) &&
    (candidate.releasedAt === undefined || dateTime(candidate.releasedAt)) &&
    (candidate.deadline === undefined || dateTime(candidate.deadline)) &&
    (candidate.expiresAt === undefined || dateTime(candidate.expiresAt)) &&
    (candidate.fencingToken === undefined ||
      /^(0|[1-9][0-9]*)$/.test(String(candidate.fencingToken))) &&
    (candidate.waitedMs === undefined || validInteger(candidate.waitedMs)) &&
    (candidate.heldMs === undefined || validInteger(candidate.heldMs)) &&
    (candidate.failureCode === undefined ||
      (typeof candidate.failureCode === 'string' &&
        /^WOML_[A-Z0-9_]+$/.test(candidate.failureCode)));
  if (!Array.isArray(value) || value.length > 100 || !value.every(lock)) {
    throw new Error(
      'The native core returned invalid Lock Inspection v1 data.'
    );
  }
  return value as unknown as readonly RustLockInspectionV1[];
}

export function observeLocksV1WithRust(
  eventStorePath: string,
  options: Pick<RustExecutorOptions, 'nativeCorePath'> = {}
): RustLockObservationV1 {
  const nativePath = options.nativeCorePath ?? defaultNativeCorePath();
  const native = loadNativeCore(nativePath);
  if (typeof native.observeWomlLocksV1 !== 'function') {
    throw new Error(
      `Native core at "${nativePath}" does not expose lock observation; rebuild the Rust addon.`
    );
  }
  const value = callRunManagementNative(() =>
    native.observeWomlLocksV1!(eventStorePath)
  );
  const count = (candidate: unknown) =>
    Number.isSafeInteger(candidate) && Number(candidate) >= 0;
  if (
    !record(value) ||
    !exactKeys(
      value,
      [
        'profile',
        'heldLocks',
        'waitingAcquisitions',
        'renewalFailures',
        'expiredTakeovers',
        'acquisitionTimeouts',
      ],
      ['oldestWaiterAgeMs']
    ) ||
    value.profile !== 'woml.lock-observation/v1' ||
    !count(value.heldLocks) ||
    !count(value.waitingAcquisitions) ||
    !count(value.renewalFailures) ||
    !count(value.expiredTakeovers) ||
    !count(value.acquisitionTimeouts) ||
    (value.oldestWaiterAgeMs !== undefined && !count(value.oldestWaiterAgeMs))
  ) {
    throw new Error(
      'The native core returned invalid Lock Observation v1 data.'
    );
  }
  return value as unknown as RustLockObservationV1;
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
      message => dispatchExecutionOrOutputProgress(message, options),
      JSON.stringify(options.resolvedSecrets ?? {}),
      options.runtimeCapabilities === undefined
        ? undefined
        : JSON.stringify(options.runtimeCapabilities)
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
      options.customNotificationHostPath ?? defaultCustomNotificationHostPath(),
      options.scriptHostPath ?? defaultScriptHostPath(),
      options.approvalBaseUrl,
      JSON.stringify(options.resolvedSecrets ?? {})
    )
    .catch(decodeNotificationError);
  return parseNotificationJourney(json);
}
