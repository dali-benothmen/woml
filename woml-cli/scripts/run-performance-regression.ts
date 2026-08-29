#!/usr/bin/env bun

import { appendFile, mkdir, writeFile } from 'node:fs/promises';
import { cpus, totalmem } from 'node:os';
import { dirname, relative, resolve } from 'node:path';

import {
  PERFORMANCE_REGRESSION_REPORT_PROFILE,
  decodePerformanceRegressionBudgets,
  evaluateInformationalJourney,
  measureHardPerformance,
  type InformationalJourneyResultV1,
} from './performance-regression';
import type { PerformanceMeasurementV1 } from './performance-measurement';

const cliRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(cliRoot, '..');
const budgetPath = resolve(
  projectRoot,
  'docs/performance-regression-budgets.v1.json'
);
const canonicalPath = resolve(
  projectRoot,
  'woml/tests/fixtures/performance-two-step-manual.woml'
);
const measurePath = resolve(import.meta.dir, 'measure-workflow.ts');

interface Arguments {
  readonly mode: 'hard' | 'report';
  readonly outputPath: string;
}

function argumentsFrom(values: readonly string[]): Arguments {
  let mode: Arguments['mode'] = 'hard';
  let outputPath = resolve(cliRoot, '.woml/performance/regression-v1.json');
  for (let index = 0; index < values.length; index += 1) {
    const value = values[index]!;
    if (value === '--mode') {
      const selected = values[++index];
      if (selected !== 'hard' && selected !== 'report') {
        throw new Error('--mode must be hard or report.');
      }
      mode = selected;
    } else if (value === '--output') {
      const selected = values[++index];
      if (selected === undefined || selected.startsWith('--')) {
        throw new Error('--output requires a file path.');
      }
      outputPath = resolve(selected);
    } else {
      throw new Error(`Unknown performance regression option: ${value}`);
    }
  }
  return { mode, outputPath };
}

async function measureJourney(
  fixture: string
): Promise<PerformanceMeasurementV1> {
  const child = Bun.spawn(
    [
      process.execPath,
      measurePath,
      resolve(projectRoot, fixture),
      '--mode',
      'manual',
      '--warmups',
      '1',
      '--iterations',
      '3',
      '--json',
    ],
    { cwd: cliRoot, stdout: 'pipe', stderr: 'pipe' }
  );
  const [stdout, stderr, exitCode] = await Promise.all([
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
    child.exited,
  ]);
  if (exitCode !== 0) {
    throw new Error(`Performance journey failed for ${fixture}:\n${stderr}`);
  }
  return JSON.parse(stdout) as PerformanceMeasurementV1;
}

function commitIdentity(): string {
  const result = Bun.spawnSync(['git', 'rev-parse', 'HEAD'], {
    cwd: projectRoot,
    stdout: 'pipe',
    stderr: 'ignore',
  });
  return result.exitCode === 0 ? result.stdout.toString().trim() : 'unknown';
}

async function writeGitHubSummary(
  hard: Awaited<ReturnType<typeof measureHardPerformance>>,
  journeys: readonly InformationalJourneyResultV1[]
): Promise<void> {
  const summaryPath = process.env.GITHUB_STEP_SUMMARY;
  if (summaryPath === undefined || summaryPath.length === 0) return;
  const lines = [
    '## WOML performance regression report',
    '',
    `Hard component gates: **${hard.passed ? 'passed' : 'failed'}**`,
    '',
    '| Journey | Cold median | Warm median | Warm p95 | Advisory |',
    '| --- | ---: | ---: | ---: | --- |',
  ];
  for (const journey of journeys) {
    lines.push(
      `| ${journey.id} | ${journey.checks.activationColdMedian.actualMs.toFixed(2)} ms | ${journey.checks.manualMedian.actualMs.toFixed(2)} ms | ${journey.checks.manualP95.actualMs.toFixed(2)} ms | ${journey.withinTargets ? 'within targets' : 'review'} |`
    );
  }
  lines.push(
    '',
    'End-to-end targets are informational and do not fail CI.',
    ''
  );
  await appendFile(summaryPath, `${lines.join('\n')}\n`);
}

const args = argumentsFrom(process.argv.slice(2));
const budgets = decodePerformanceRegressionBudgets(
  await Bun.file(budgetPath).json()
);
const hard = await measureHardPerformance(canonicalPath, budgets);
if (!hard.passed) {
  process.stderr.write(
    `${JSON.stringify({ profile: budgets.profile, hard }, null, 2)}\n`
  );
  throw new Error(
    'A deterministic WOML performance regression budget was exceeded.'
  );
}

if (args.mode === 'hard') {
  process.stdout.write(
    `${JSON.stringify({ profile: budgets.profile, hard }, null, 2)}\n`
  );
  process.exit(0);
}

const journeys: InformationalJourneyResultV1[] = [];
for (const [id, budget] of Object.entries(budgets.informational.journeys)) {
  const measurement = await measureJourney(budget.fixture);
  journeys.push(
    evaluateInformationalJourney(
      id,
      budget,
      budgets.informational.activationColdMedianMsMax,
      measurement
    )
  );
}
const cpu = cpus();
const report = {
  profile: PERFORMANCE_REGRESSION_REPORT_PROFILE,
  createdAt: new Date().toISOString(),
  commit: commitIdentity(),
  budgetProfile: budgets.profile,
  environment: {
    platform: process.platform,
    architecture: process.arch,
    bunVersion: Bun.version,
    cpuModel: cpu[0]?.model ?? 'unknown',
    logicalCpuCount: Math.max(1, cpu.length),
    memoryBytes: totalmem(),
  },
  hard,
  journeys,
  advisory: {
    withinTargets: journeys.every(journey => journey.withinTargets),
    blocking: false,
  },
};
await mkdir(dirname(args.outputPath), { recursive: true });
await writeFile(args.outputPath, `${JSON.stringify(report, null, 2)}\n`, {
  mode: 0o600,
});
await writeGitHubSummary(hard, journeys);
process.stdout.write(
  `${JSON.stringify(
    {
      profile: report.profile,
      outputPath: relative(projectRoot, args.outputPath),
      hardPassed: hard.passed,
      advisoryWithinTargets: report.advisory.withinTargets,
      journeys: journeys.map(journey => ({
        id: journey.id,
        withinTargets: journey.withinTargets,
        coldMedianMs: journey.checks.activationColdMedian.actualMs,
        warmMedianMs: journey.checks.manualMedian.actualMs,
        warmP95Ms: journey.checks.manualP95.actualMs,
      })),
    },
    null,
    2
  )}\n`
);
