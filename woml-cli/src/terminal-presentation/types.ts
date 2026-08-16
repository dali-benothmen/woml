export const RUN_PRESENTATION_PROFILE = 'woml.run-presentation/v1' as const;

export type JsonValue =
  | null
  | boolean
  | number
  | string
  | readonly JsonValue[]
  | { readonly [key: string]: JsonValue };

export type TriggerPresentationType =
  | 'manual'
  | 'webhook'
  | 'slack'
  | 'telegram'
  | 'discord'
  | 'schedule'
  | 'interval'
  | 'event';

export interface TriggerPresentationV1 {
  readonly id: string;
  readonly type: TriggerPresentationType;
  readonly label?: string;
  readonly method?: string;
  readonly url?: string;
  readonly example?: string;
  readonly schedule?: string;
  readonly timezone?: string;
  readonly nextDueAt?: string;
  readonly interval?: string;
  readonly event?: string;
  readonly workspace?: string;
  readonly scope?: string;
  readonly warning?: string;
}

export interface WorkflowPresentationV1 {
  readonly id: string;
  readonly name?: string;
  readonly description?: string;
  readonly version?: string;
  readonly definitionHash: string;
  readonly triggers: readonly TriggerPresentationV1[];
}

export type RunPresentationStatus =
  | 'queued'
  | 'running'
  | 'waiting'
  | 'retrying'
  | 'cancelling'
  | 'finalizing'
  | 'succeeded'
  | 'failed'
  | 'cancelled'
  | 'timed_out';

export type StepPresentationStatus =
  | 'queued'
  | 'running'
  | 'waiting'
  | 'retrying'
  | 'succeeded'
  | 'failed'
  | 'cancelled'
  | 'timed_out'
  | 'skipped';

export type StepPresentationKind =
  | 'step'
  | 'script'
  | 'custom_step'
  | 'switch'
  | 'choose'
  | 'parallel'
  | 'fork'
  | 'branch'
  | 'approval'
  | 'workflow_call'
  | 'workflow_start';

export interface PresentationFailureV1 {
  readonly code: string;
  readonly message: string;
  readonly kind?: string;
  readonly retryable?: boolean;
}

export interface StepPresentationV1 {
  readonly id: string;
  readonly name?: string;
  readonly description?: string;
  readonly kind: StepPresentationKind;
  readonly status: StepPresentationStatus;
  readonly depth: number;
  readonly startedAt?: string;
  readonly completedAt?: string;
  readonly durationMs?: number;
  readonly attempts: number;
  readonly detail?: string;
  readonly result?: JsonValue;
  readonly resultTruncated?: boolean;
  readonly failure?: PresentationFailureV1;
}

export type LifecycleHookPresentation =
  | 'on-start'
  | 'on-success'
  | 'on-error'
  | 'on-cancel'
  | 'on-complete'
  | 'on-step-start'
  | 'on-step-success'
  | 'on-step-failure'
  | 'on-step-complete';

export interface LifecyclePresentationV1 {
  readonly hook: LifecycleHookPresentation;
  readonly status: StepPresentationStatus;
  readonly durationMs?: number;
  readonly provider?: string;
  readonly detail?: string;
  readonly failure?: PresentationFailureV1;
}

export interface RunSummaryV1 {
  readonly succeeded: number;
  readonly failed: number;
  readonly skipped: number;
  readonly cancelled: number;
  readonly total: number;
}

export interface RunPresentationV1 {
  readonly profile: typeof RUN_PRESENTATION_PROFILE;
  readonly workflow: WorkflowPresentationV1;
  readonly runId: string;
  readonly trigger: {
    readonly id: string;
    readonly type: TriggerPresentationType;
  };
  readonly status: RunPresentationStatus;
  readonly admittedAt: string;
  readonly startedAt?: string;
  readonly completedAt?: string;
  readonly durationMs?: number;
  readonly steps: readonly StepPresentationV1[];
  readonly summary: RunSummaryV1;
  readonly lifecycle: readonly LifecyclePresentationV1[];
  readonly result?: JsonValue;
  readonly resultTruncated?: boolean;
  readonly failure?: PresentationFailureV1;
  readonly warnings: readonly PresentationFailureV1[];
}

export interface RunPresentationListV1 {
  readonly profile: 'woml.run-presentation-list/v1';
  readonly workflowId: string;
  readonly runs: readonly RunPresentationV1[];
}

export type HumanPresentationFormat = 'tty' | 'plain';
export type PresentationFormat = HumanPresentationFormat | 'json';
export type ColorMode = 'auto' | 'always' | 'never';

export interface PresentationRenderOptions {
  readonly format?: PresentationFormat;
  readonly color?: ColorMode;
  readonly isTTY?: boolean;
  readonly width?: number;
  readonly unicode?: boolean;
  readonly locale?: string;
  readonly timeZone?: string;
  readonly environment?: Readonly<Record<string, string | undefined>>;
  readonly fullResultCommand?: (runId: string) => string;
  /** Temporary/runtime-specific manual instruction; the frozen default is interactive. */
  readonly manualInstruction?: string;
}
