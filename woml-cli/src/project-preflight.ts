import type { ResolvedWomlProjectV1 } from './project';

export const WOML_PROJECT_CHECK_PROFILE = 'woml.project-check/v1' as const;

export type ProjectCheckStatus = 'ready' | 'degraded' | 'blocked';
export type ProjectSubsystemStatus =
  | 'ready'
  | 'degraded'
  | 'blocked'
  | 'not-required';

export interface ProjectCheckDiagnosticV1 {
  readonly code: string;
  readonly severity: 'warning' | 'error';
  readonly message: string;
  readonly correctiveAction: string;
  readonly sourcePath?: string;
  readonly line?: number;
  readonly column?: number;
}

export interface ProjectCheckWorkflowV1 {
  readonly id: string;
  readonly sourcePath: string;
  readonly status: 'ready' | 'blocked';
  readonly requiredCapabilities: readonly string[];
}

export interface ProjectCheckSubsystemV1 {
  readonly id: string;
  readonly status: ProjectSubsystemStatus;
  readonly required: boolean;
  readonly digest?: string;
}

export interface WomlProjectCheckV1 {
  readonly profile: typeof WOML_PROJECT_CHECK_PROFILE;
  readonly version: 1;
  readonly status: ProjectCheckStatus;
  readonly project: {
    readonly name: string;
    readonly digest: string;
    readonly manifestPath: string;
  };
  readonly workflows: readonly ProjectCheckWorkflowV1[];
  readonly subsystems: readonly ProjectCheckSubsystemV1[];
  readonly diagnostics: readonly ProjectCheckDiagnosticV1[];
}

const MAX_DIAGNOSTICS = 1024;

function lexical(left: string, right: string): number {
  return left.localeCompare(right);
}

export function createWomlProjectCheck(
  project: ResolvedWomlProjectV1,
  workflows: readonly ProjectCheckWorkflowV1[],
  subsystems: readonly ProjectCheckSubsystemV1[],
  diagnostics: readonly ProjectCheckDiagnosticV1[] = []
): WomlProjectCheckV1 {
  const boundedDiagnostics = diagnostics.slice(0, MAX_DIAGNOSTICS);
  const blocked =
    workflows.some(workflow => workflow.status === 'blocked') ||
    subsystems.some(subsystem => subsystem.status === 'blocked') ||
    boundedDiagnostics.some(diagnostic => diagnostic.severity === 'error');
  const degraded =
    subsystems.some(subsystem => subsystem.status === 'degraded') ||
    boundedDiagnostics.some(diagnostic => diagnostic.severity === 'warning');
  return {
    profile: WOML_PROJECT_CHECK_PROFILE,
    version: 1,
    status: blocked ? 'blocked' : degraded ? 'degraded' : 'ready',
    project: {
      name: project.name,
      digest: project.digest,
      manifestPath: project.manifestPath,
    },
    workflows: [...workflows]
      .map(workflow => ({
        ...workflow,
        requiredCapabilities: [...new Set(workflow.requiredCapabilities)].sort(
          lexical
        ),
      }))
      .sort((left, right) => lexical(left.id, right.id)),
    subsystems: [...subsystems].sort((left, right) => lexical(left.id, right.id)),
    diagnostics: boundedDiagnostics,
  };
}

export function formatWomlProjectCheck(report: WomlProjectCheckV1): string {
  const mark = (status: ProjectSubsystemStatus | ProjectCheckStatus) =>
    status === 'ready' || status === 'not-required'
      ? '✓'
      : status === 'degraded'
        ? '●'
        : '✗';
  const lines = [
    `${mark(report.status)} Project       ${report.project.name}`,
    `${mark(report.workflows.some(item => item.status === 'blocked') ? 'blocked' : 'ready')} Workflows     ${report.workflows.length} compiled`,
  ];
  for (const subsystem of report.subsystems) {
    lines.push(
      `${mark(subsystem.status)} ${subsystem.id.padEnd(13)} ${subsystem.status}`
    );
  }
  for (const diagnostic of report.diagnostics) {
    lines.push(
      `${diagnostic.severity === 'error' ? '✗' : '!'} ${diagnostic.code}: ${diagnostic.message}`,
      `  ${diagnostic.correctiveAction}`
    );
  }
  lines.push('', report.status === 'blocked' ? 'Blocked' : 'Ready to run');
  return `${lines.join('\n')}\n`;
}

