import type {
  RustArtifactInspectionV1,
  RustCorrectionInspectionV1,
  RustLockInspectionV1,
  RustRemoteDispatchInspectionV1,
  RustRunInspectionV2,
  RustWaitInspectionV1,
} from './rust-executor';
import { sanitizeTerminalText } from './terminal-presentation';

export const COMBINED_RUN_PRESENTATION_PROFILE =
  'woml.combined-run-presentation/v1' as const;
export const PROJECT_ACTIVATION_PRESENTATION_PROFILE =
  'woml.project-activation-presentation/v1' as const;

export type CombinedRunState =
  | 'queued'
  | 'running'
  | 'waiting-for-approval'
  | 'waiting-for-signal'
  | 'waiting-for-time'
  | 'waiting-for-lock'
  | 'queued-for-target'
  | 'executing-remotely'
  | 'cancellation-requested'
  | 'compensating'
  | 'cleaning-up'
  | 'manual-recovery'
  | 'ambiguous-remote-outcome'
  | 'completed'
  | 'failed'
  | 'cancelled';

export interface ProjectActivationPresentationV1 {
  readonly profile: typeof PROJECT_ACTIVATION_PRESENTATION_PROFILE;
  readonly version: 1;
  readonly project: string;
  readonly digest: string;
  readonly workflows: readonly string[];
  readonly statePath: string;
  readonly policy?: string;
  readonly targets: readonly string[];
  readonly openTelemetry: 'enabled' | 'disabled';
  readonly status: 'ready';
}

export interface CombinedRunPresentationV1 {
  readonly profile: typeof COMBINED_RUN_PRESENTATION_PROFILE;
  readonly version: 1;
  readonly runId: string;
  readonly workflowId: string;
  readonly state: CombinedRunState;
  readonly detail: string;
  readonly nextCommands: readonly string[];
}

export interface CombinedRunPresentationInput {
  readonly inspection: RustRunInspectionV2;
  readonly statePath: string;
  readonly waits?: RustWaitInspectionV1;
  readonly locks?: readonly RustLockInspectionV1[];
  readonly correction?: RustCorrectionInspectionV1;
  readonly artifacts?: readonly RustArtifactInspectionV1[];
  readonly remoteDispatch?: RustRemoteDispatchInspectionV1;
}

function quote(value: string): string {
  return JSON.stringify(sanitizeTerminalText(value).replaceAll('\n', ' '));
}

function present(value: string): string {
  return sanitizeTerminalText(value).replaceAll('\n', ' ').trim();
}

function terminalState(
  inspection: RustRunInspectionV2
): Pick<CombinedRunPresentationV1, 'state' | 'detail'> | undefined {
  if (inspection.status === 'succeeded') {
    return { state: 'completed', detail: 'Workflow completed successfully' };
  }
  if (inspection.status === 'cancelled') {
    return { state: 'cancelled', detail: 'Workflow was cancelled' };
  }
  if (inspection.status === 'failed') {
    return { state: 'failed', detail: 'Workflow failed' };
  }
  return undefined;
}

function activeApproval(inspection: RustRunInspectionV2) {
  return inspection.approvals?.items.find(item =>
    ['pending', 'waiting', 'requested'].includes(item.status)
  );
}

function pendingWait(waits: RustWaitInspectionV1 | undefined) {
  return waits?.waits.find(wait => wait.status === 'pending');
}

function activeLock(locks: readonly RustLockInspectionV1[] | undefined) {
  return locks?.find(lock => lock.status === 'waiting' || lock.status === 'held');
}

function deriveState(
  input: CombinedRunPresentationInput
): Pick<CombinedRunPresentationV1, 'state' | 'detail'> {
  const { inspection, correction, remoteDispatch } = input;
  if (inspection.cancellation.requested) {
    return {
      state: 'cancellation-requested',
      detail: 'Cancellation requested; owned work is settling safely',
    };
  }
  if (remoteDispatch?.ambiguous === true) {
    return {
      state: 'ambiguous-remote-outcome',
      detail: `Remote dispatch ${present(remoteDispatch.dispatchId)} needs operator reconciliation`,
    };
  }
  if (correction?.phase === 'compensating') {
    return { state: 'compensating', detail: 'Reversing completed saga effects' };
  }
  if (correction?.phase === 'cleanup') {
    return { state: 'cleaning-up', detail: 'Running mandatory saga cleanup' };
  }
  if (correction?.phase === 'manual_recovery') {
    return {
      state: 'manual-recovery',
      detail: 'A corrective action needs an operator decision',
    };
  }
  const approval = activeApproval(inspection);
  if (approval !== undefined) {
    return {
      state: 'waiting-for-approval',
      detail: `${present(approval.approvalId)} · ${approval.received}/${approval.required} approvals`,
    };
  }
  const wait = pendingWait(input.waits);
  if (wait !== undefined) {
    if (wait.kind === 'signal') {
      return {
        state: 'waiting-for-signal',
        detail: `${present(wait.waitId)} · ${(wait.signals ?? []).map(present).join(', ')}`,
      };
    }
    return {
      state: 'waiting-for-time',
      detail: `${present(wait.waitId)}${wait.deadline === undefined ? '' : ` · until ${present(wait.deadline)}`}`,
    };
  }
  const lock = activeLock(input.locks);
  if (lock?.status === 'waiting') {
    return {
      state: 'waiting-for-lock',
      detail: `${present(lock.lockId)} · ${present(lock.key)}`,
    };
  }
  if (remoteDispatch !== undefined) {
    if (
      ['offered', 'accepted', 'queued', 'pending'].includes(
        remoteDispatch.state
      )
    ) {
      return {
        state: 'queued-for-target',
        detail: `${present(remoteDispatch.target)} · ${present(remoteDispatch.dispatchId)}`,
      };
    }
    if (['started', 'running', 'cancelling'].includes(remoteDispatch.state)) {
      return {
        state: 'executing-remotely',
        detail: `${present(remoteDispatch.target)}${remoteDispatch.workerId === undefined ? '' : ` · ${present(remoteDispatch.workerId)}`}`,
      };
    }
  }
  const terminal = terminalState(inspection);
  if (terminal !== undefined) return terminal;
  if (inspection.policy?.waitingFor !== undefined) {
    return {
      state: 'queued',
      detail: `Waiting for ${inspection.policy.waitingFor.replaceAll('_', ' ')}`,
    };
  }
  return { state: 'running', detail: 'Workflow execution is in progress' };
}

function nextCommands(input: CombinedRunPresentationInput): readonly string[] {
  const { inspection, statePath, correction, remoteDispatch } = input;
  const state = quote(statePath);
  const commands = [
    `woml get ${quote(inspection.runId)} --state ${state}`,
    `woml logs ${quote(inspection.runId)} --state ${state}`,
  ];
  const wait = pendingWait(input.waits);
  if (wait?.kind === 'signal' && wait.signals?.[0] !== undefined) {
    commands.push(
      `woml signal ${quote(wait.signals[0])} --run ${quote(inspection.runId)} --wait ${quote(wait.waitId)} --id <unique-id> --correlation <value> --payload <json-file> --state ${state}`
    );
  }
  const unresolved = correction?.unresolvedActions[0];
  if (unresolved !== undefined) {
    commands.push(
      `woml recover ${quote(inspection.runId)} --retry ${quote(unresolved.actionId)} --state ${state}`
    );
  }
  const artifact = input.artifacts?.[0];
  if (artifact !== undefined) {
    commands.push(
      `woml artifacts export ${quote(artifact.artifact.id)} --output ${quote(`./${artifact.artifact.name}`)} --artifact-policy <path> --state ${state}`
    );
  }
  if (remoteDispatch !== undefined) {
    commands.push(
      `woml dispatch inspect ${quote(remoteDispatch.dispatchId)} --state ${state}`
    );
    if (remoteDispatch.ambiguous) {
      commands.push(
        `woml dispatch reconcile ${quote(remoteDispatch.dispatchId)} --action record-failure --request-id <unique-id> --reason <verified-evidence> --state ${state}`
      );
    }
  }
  return commands.slice(0, 8);
}

export function createProjectActivationPresentation(input: {
  readonly project: string;
  readonly digest: string;
  readonly workflows: readonly string[];
  readonly statePath: string;
  readonly policy?: string;
  readonly targets?: readonly string[];
  readonly openTelemetry: boolean;
}): ProjectActivationPresentationV1 {
  return {
    profile: PROJECT_ACTIVATION_PRESENTATION_PROFILE,
    version: 1,
    project: present(input.project),
    digest: present(input.digest),
    workflows: [...new Set(input.workflows.map(present))].sort(),
    statePath: present(input.statePath),
    ...(input.policy === undefined ? {} : { policy: present(input.policy) }),
    targets: [...new Set((input.targets ?? []).map(present))].sort(),
    openTelemetry: input.openTelemetry ? 'enabled' : 'disabled',
    status: 'ready',
  };
}

export function renderProjectActivationPresentation(
  presentation: ProjectActivationPresentationV1,
  json = false
): string {
  if (json) return `${JSON.stringify(presentation)}\n`;
  const digest = presentation.digest.replace(/^sha256:/u, '').slice(0, 12);
  return [
    `PROJECT  ${presentation.project} · ${digest}`,
    `Workflow${presentation.workflows.length === 1 ? ' ' : 's'} ${presentation.workflows.join(', ')}`,
    `State     ${presentation.statePath}`,
    `Policy    ${presentation.policy ?? 'not configured'}`,
    `Target    ${presentation.targets.length === 0 ? 'local' : presentation.targets.join(', ')}`,
    `Telemetry ${presentation.openTelemetry}`,
    'Status    Ready',
    '',
  ].join('\n');
}

export function createCombinedRunPresentation(
  input: CombinedRunPresentationInput
): CombinedRunPresentationV1 {
  const state = deriveState(input);
  return {
    profile: COMBINED_RUN_PRESENTATION_PROFILE,
    version: 1,
    runId: present(input.inspection.runId),
    workflowId: present(input.inspection.workflowId),
    ...state,
    nextCommands: nextCommands(input),
  };
}

export function renderCombinedRunPresentation(
  presentation: CombinedRunPresentationV1,
  json = false
): string {
  if (json) return `${JSON.stringify(presentation)}\n`;
  const lines = [
    `Run: ${presentation.runId}`,
    `Workflow: ${presentation.workflowId}`,
    `Current: ${presentation.state.replaceAll('-', ' ')} · ${presentation.detail}`,
  ];
  if (presentation.nextCommands.length > 0) {
    lines.push('Next safe actions:');
    for (const command of presentation.nextCommands) lines.push(`  ${command}`);
  }
  return `${lines.join('\n')}\n`;
}
