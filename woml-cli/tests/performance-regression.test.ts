import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import Ajv2020 from 'ajv/dist/2020';

import {
  decodePerformanceRegressionBudgets,
  evaluateInformationalJourney,
} from '../scripts/performance-regression';
import {
  PERFORMANCE_MEASUREMENT_PROFILE,
  performanceMetric,
} from '../scripts/performance-measurement';

const projectRoot = resolve(import.meta.dir, '../..');
const budgetPath = resolve(
  projectRoot,
  'docs/performance-regression-budgets.v1.json'
);
const budgetsValue = JSON.parse(readFileSync(budgetPath, 'utf8'));

describe('PERF8 performance regression contracts', () => {
  test('validates the versioned budgets and names every owner', () => {
    const schema = JSON.parse(
      readFileSync(
        resolve(
          projectRoot,
          'docs/schemas/performance-regression-budgets.v1.schema.json'
        ),
        'utf8'
      )
    );
    const validate = new Ajv2020({ strict: false, allErrors: true }).compile(
      schema
    );
    expect(
      validate(budgetsValue),
      JSON.stringify(validate.errors, null, 2)
    ).toBe(true);
    const budgets = decodePerformanceRegressionBudgets(budgetsValue);
    expect(Object.keys(budgets.owners).sort()).toEqual([
      'ci',
      'compiler',
      'presentation',
      'runtime',
    ]);
    expect(budgets.rationale.hard.length).toBeGreaterThan(40);
    expect(budgets.rationale.informational.length).toBeGreaterThan(40);
  });

  test('connects the existing terminal and production budget contracts', () => {
    const budgets = decodePerformanceRegressionBudgets(budgetsValue);
    const terminal = JSON.parse(
      readFileSync(
        resolve(
          projectRoot,
          'examples/terminalExperience/performance-budgets.v1.json'
        ),
        'utf8'
      )
    );
    const production = JSON.parse(
      readFileSync(
        resolve(projectRoot, 'examples/production/performance-budgets.v1.json'),
        'utf8'
      )
    );
    expect(terminal.profile).toBe(budgets.existingContracts.terminal);
    expect(production.profile).toBe(budgets.existingContracts.production);
    expect(terminal.regressionPolicy).toBe(budgets.profile);
    expect(production.regressionPolicy).toBe(budgets.profile);
  });

  test('marks noisy end-to-end misses for review without turning them into failures', () => {
    const budgets = decodePerformanceRegressionBudgets(budgetsValue);
    const budget = budgets.informational.journeys.canonical!;
    const measurement = {
      profile: PERFORMANCE_MEASUREMENT_PROFILE,
      createdAt: '2026-08-29T00:00:00.000Z',
      fixture: { path: budget.fixture, sha256: 'a'.repeat(64) },
      environment: {
        platform: 'linux',
        architecture: 'x64',
        bunVersion: '1.3.14',
        cpuModel: 'test',
        logicalCpuCount: 2,
        memoryBytes: 1024,
        nativeBuild: 'release' as const,
        cliArtifact: 'built' as const,
      },
      parameters: { mode: 'manual' as const, warmups: 1, iterations: 3 },
      metrics: [
        performanceMetric(
          'activation.cold',
          'cold',
          { start: 'start', end: 'ready' },
          [budgets.informational.activationColdMedianMsMax + 1]
        ),
        performanceMetric(
          'manual.visible',
          'warm',
          { start: 'trigger', end: 'result' },
          [Math.max(budget.manualMedianMsMax, budget.manualP95MsMax) + 1]
        ),
      ],
    };
    const result = evaluateInformationalJourney(
      'canonical',
      budget,
      budgets.informational.activationColdMedianMsMax,
      measurement
    );
    expect(result.withinTargets).toBe(false);
    expect(
      Object.values(result.checks).every(check => !check.withinTarget)
    ).toBe(true);
  });

  test('rejects malformed budgets before a benchmark starts', () => {
    expect(() =>
      decodePerformanceRegressionBudgets({
        ...budgetsValue,
        hard: { ...budgetsValue.hard, isolatedWorkersPerScript: 0 },
      })
    ).toThrow('budget fields');
  });
});
