import { compileWoml, parseWoml } from '@woml/compiler';

import {
  performanceProfileEnabled,
  profileSync,
} from '../src/performance-profiler';
import {
  summarizeSamples,
  type PerformanceMeasurementV1,
  type PerformanceSummaryV1,
} from './performance-measurement';

export const PERFORMANCE_REGRESSION_BUDGET_PROFILE =
  'woml.performance-regression-budgets/v1' as const;
export const PERFORMANCE_REGRESSION_REPORT_PROFILE =
  'woml.performance-regression-report/v1' as const;

export interface PerformanceJourneyBudgetV1 {
  readonly fixture: string;
  readonly manualMedianMsMax: number;
  readonly manualP95MsMax: number;
}

export interface PerformanceRegressionBudgetsV1 {
  readonly profile: typeof PERFORMANCE_REGRESSION_BUDGET_PROFILE;
  readonly owners: Readonly<
    Record<'compiler' | 'runtime' | 'presentation' | 'ci', string>
  >;
  readonly rationale: Readonly<Record<'hard' | 'informational', string>>;
  readonly hard: {
    readonly canonicalWorkflowId: string;
    readonly canonicalNodes: number;
    readonly canonicalEdges: number;
    readonly measuredRuns: number;
    readonly maxScriptHostSpawns: number;
    readonly maxScriptHostShutdowns: number;
    readonly isolatedWorkersPerScript: 1;
    readonly compilerMedianMsMax: number;
    readonly compilerP95MsMax: number;
    readonly profilerDisabledOverheadNsMax: number;
  };
  readonly informational: {
    readonly activationColdMedianMsMax: number;
    readonly journeys: Readonly<Record<string, PerformanceJourneyBudgetV1>>;
  };
  readonly existingContracts: {
    readonly terminal: 'woml.terminal-performance-budgets/v1';
    readonly production: 'woml.production-performance-budgets/v1';
  };
}

export interface HardPerformanceResultV1 {
  readonly compiler: PerformanceSummaryV1;
  readonly profilerDisabledOverheadNs: number;
  readonly checks: {
    readonly compilerMedian: boolean;
    readonly compilerP95: boolean;
    readonly profilerDisabledOverhead: boolean;
  };
  readonly passed: boolean;
}

export interface InformationalCheckV1 {
  readonly actualMs: number;
  readonly targetMs: number;
  readonly withinTarget: boolean;
}

export interface InformationalJourneyResultV1 {
  readonly id: string;
  readonly fixture: string;
  readonly measurement: PerformanceMeasurementV1;
  readonly checks: {
    readonly activationColdMedian: InformationalCheckV1;
    readonly manualMedian: InformationalCheckV1;
    readonly manualP95: InformationalCheckV1;
  };
  readonly withinTargets: boolean;
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function positive(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value) && value > 0;
}

export function decodePerformanceRegressionBudgets(
  value: unknown
): PerformanceRegressionBudgetsV1 {
  if (
    !record(value) ||
    value.profile !== PERFORMANCE_REGRESSION_BUDGET_PROFILE ||
    !record(value.owners) ||
    !record(value.rationale) ||
    !record(value.hard) ||
    !record(value.informational) ||
    !record(value.informational.journeys) ||
    !record(value.existingContracts)
  ) {
    throw new Error('Invalid WOML performance regression budget contract.');
  }
  const owners = value.owners;
  const requiredOwners = ['compiler', 'runtime', 'presentation', 'ci'];
  if (
    requiredOwners.some(
      owner => typeof owners[owner] !== 'string' || owners[owner].length === 0
    ) ||
    typeof value.rationale.hard !== 'string' ||
    value.rationale.hard.length === 0 ||
    typeof value.rationale.informational !== 'string' ||
    value.rationale.informational.length === 0 ||
    typeof value.hard.canonicalWorkflowId !== 'string' ||
    value.hard.canonicalWorkflowId.length === 0 ||
    !Number.isSafeInteger(value.hard.canonicalNodes) ||
    Number(value.hard.canonicalNodes) < 0 ||
    !Number.isSafeInteger(value.hard.canonicalEdges) ||
    Number(value.hard.canonicalEdges) < 0 ||
    !Number.isSafeInteger(value.hard.measuredRuns) ||
    Number(value.hard.measuredRuns) < 2 ||
    !Number.isSafeInteger(value.hard.maxScriptHostSpawns) ||
    Number(value.hard.maxScriptHostSpawns) < 1 ||
    !Number.isSafeInteger(value.hard.maxScriptHostShutdowns) ||
    Number(value.hard.maxScriptHostShutdowns) < 1 ||
    value.hard.isolatedWorkersPerScript !== 1 ||
    !positive(value.hard.compilerMedianMsMax) ||
    !positive(value.hard.compilerP95MsMax) ||
    !positive(value.hard.profilerDisabledOverheadNsMax) ||
    !positive(value.informational.activationColdMedianMsMax) ||
    value.existingContracts.terminal !==
      'woml.terminal-performance-budgets/v1' ||
    value.existingContracts.production !==
      'woml.production-performance-budgets/v1'
  ) {
    throw new Error('Invalid WOML performance regression budget fields.');
  }
  if (Object.keys(value.informational.journeys).length === 0) {
    throw new Error('At least one informational journey budget is required.');
  }
  for (const journey of Object.values(value.informational.journeys)) {
    if (
      !record(journey) ||
      typeof journey.fixture !== 'string' ||
      journey.fixture.length === 0 ||
      !positive(journey.manualMedianMsMax) ||
      !positive(journey.manualP95MsMax)
    ) {
      throw new Error('Invalid WOML informational journey budget.');
    }
  }
  return value as unknown as PerformanceRegressionBudgetsV1;
}

function elapsedSamples(
  operation: () => void,
  warmups: number,
  iterations: number
): number[] {
  for (let index = 0; index < warmups; index += 1) operation();
  const samples: number[] = [];
  for (let index = 0; index < iterations; index += 1) {
    const startedAt = performance.now();
    operation();
    samples.push(performance.now() - startedAt);
  }
  return samples;
}

function disabledProfilerOverheadNs(): number {
  if (performanceProfileEnabled()) {
    throw new Error(
      'The deterministic regression gate requires WOML profiling to be disabled.'
    );
  }
  const batches = 15;
  const iterations = 20_000;
  const direct: number[] = [];
  const wrapped: number[] = [];
  let checksum = 0;
  const operation = () => {
    checksum = (checksum + 1) | 0;
  };
  for (let batch = 0; batch < batches; batch += 1) {
    let startedAt = performance.now();
    for (let index = 0; index < iterations; index += 1) operation();
    direct.push((performance.now() - startedAt) / iterations);

    startedAt = performance.now();
    for (let index = 0; index < iterations; index += 1) {
      profileSync('cli', 'regression.disabled', operation);
    }
    wrapped.push((performance.now() - startedAt) / iterations);
  }
  if (checksum === 0)
    throw new Error('The profiler benchmark was optimized away.');
  return Math.max(
    0,
    (summarizeSamples(wrapped).medianMs - summarizeSamples(direct).medianMs) *
      1_000_000
  );
}

export async function measureHardPerformance(
  sourcePath: string,
  budgets: PerformanceRegressionBudgetsV1
): Promise<HardPerformanceResultV1> {
  const source = await Bun.file(sourcePath).text();
  const compiler = summarizeSamples(
    elapsedSamples(
      () => {
        const workflow = compileWoml(parseWoml(source, { file: sourcePath }));
        if (workflow.workflowId !== budgets.hard.canonicalWorkflowId) {
          throw new Error(
            'The canonical performance fixture compiled to an unexpected workflow.'
          );
        }
      },
      5,
      30
    )
  );
  const profilerDisabledOverheadNs = disabledProfilerOverheadNs();
  const checks = {
    compilerMedian: compiler.medianMs <= budgets.hard.compilerMedianMsMax,
    compilerP95: compiler.p95Ms <= budgets.hard.compilerP95MsMax,
    profilerDisabledOverhead:
      profilerDisabledOverheadNs <= budgets.hard.profilerDisabledOverheadNsMax,
  };
  return {
    compiler,
    profilerDisabledOverheadNs,
    checks,
    passed: Object.values(checks).every(Boolean),
  };
}

function informationalCheck(
  actualMs: number,
  targetMs: number
): InformationalCheckV1 {
  return { actualMs, targetMs, withinTarget: actualMs <= targetMs };
}

export function evaluateInformationalJourney(
  id: string,
  budget: PerformanceJourneyBudgetV1,
  activationColdMedianMsMax: number,
  measurement: PerformanceMeasurementV1
): InformationalJourneyResultV1 {
  const activation = measurement.metrics.find(
    metric => metric.name === 'activation.cold'
  );
  const manual = measurement.metrics.find(
    metric => metric.name === 'manual.visible'
  );
  if (activation === undefined || manual === undefined) {
    throw new Error(`Informational journey ${id} is missing required metrics.`);
  }
  const checks = {
    activationColdMedian: informationalCheck(
      activation.summary.medianMs,
      activationColdMedianMsMax
    ),
    manualMedian: informationalCheck(
      manual.summary.medianMs,
      budget.manualMedianMsMax
    ),
    manualP95: informationalCheck(manual.summary.p95Ms, budget.manualP95MsMax),
  };
  return {
    id,
    fixture: budget.fixture,
    measurement,
    checks,
    withinTargets: Object.values(checks).every(check => check.withinTarget),
  };
}
