#!/usr/bin/env bun

import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import { compileWoml, parseWoml } from '@woml/compiler';

import {
  executeWorkflowWithRustDurable,
  inspectRunPresentationWithRust,
} from '../src/rust-executor';
import { ForegroundPresentation } from '../src/foreground-presentation';
import { renderRunPresentation } from '../src/terminal-presentation';

const packageRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(packageRoot, '..');
const nativeCorePath = resolve(
  packageRoot,
  'dist',
  `woml-core.${process.platform}-${process.arch}.node`
);
const scriptHostPath = resolve(packageRoot, 'dist/script-host.js');
const fixturePath = resolve(
  projectRoot,
  'woml/tests/fixtures/performance-eight-step-manual.woml'
);
const textEncoder = new TextEncoder();

function median(values: readonly number[]): number {
  const sorted = [...values].sort((left, right) => left - right);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0
    ? (sorted[middle - 1]! + sorted[middle]!) / 2
    : sorted[middle]!;
}

function measure(operation: () => string): {
  readonly medianMs: number;
  readonly minimumMs: number;
  readonly maximumMs: number;
  readonly outputBytes: number;
} {
  const warmups = 100;
  const batches = 20;
  const iterations = 200;
  let outputBytes = 0;
  for (let index = 0; index < warmups; index += 1) {
    outputBytes = textEncoder.encode(operation()).byteLength;
  }
  const samples: number[] = [];
  for (let batch = 0; batch < batches; batch += 1) {
    const startedAt = performance.now();
    for (let index = 0; index < iterations; index += 1) {
      outputBytes = textEncoder.encode(operation()).byteLength;
    }
    samples.push((performance.now() - startedAt) / iterations);
  }
  return {
    medianMs: median(samples),
    minimumMs: Math.min(...samples),
    maximumMs: Math.max(...samples),
    outputBytes,
  };
}

function measureJourney(operation: () => number): {
  readonly medianMs: number;
  readonly minimumMs: number;
  readonly maximumMs: number;
  readonly outputBytes: number;
} {
  const warmups = 3;
  const batches = 10;
  const iterations = 10;
  let outputBytes = 0;
  for (let index = 0; index < warmups; index += 1) outputBytes = operation();
  const samples: number[] = [];
  for (let batch = 0; batch < batches; batch += 1) {
    const startedAt = performance.now();
    for (let index = 0; index < iterations; index += 1) outputBytes = operation();
    samples.push((performance.now() - startedAt) / iterations);
  }
  return {
    medianMs: median(samples),
    minimumMs: Math.min(...samples),
    maximumMs: Math.max(...samples),
    outputBytes,
  };
}

const temporaryRoot = await mkdtemp(join(tmpdir(), 'woml-presentation-profile-'));
try {
  const source = await Bun.file(fixturePath).text();
  const workflow = compileWoml(parseWoml(source, { file: fixturePath }));
  const statePath = join(temporaryRoot, 'state.sqlite');
  const execution = await executeWorkflowWithRustDurable(workflow, statePath, {
    nativeCorePath,
    scriptHostPath,
  });
  const presentation = inspectRunPresentationWithRust(
    statePath,
    execution.runId,
    { nativeCorePath }
  );
  const common = { width: 100, unicode: true, timeZone: 'UTC' } as const;
  const accepted = {
    contract: 'woml.trigger-progress' as const,
    contractVersion: 1 as const,
    type: 'occurrence_accepted' as const,
    workflowId: presentation.workflow.id,
    triggerId: presentation.trigger.id,
    triggerHandler: 'trigger.manual',
    occurrenceId: 'occurrence_performance',
    runId: presentation.runId,
    duplicate: false,
    occurredAt: presentation.admittedAt,
  };
  const started = {
    contract: 'woml.trigger-progress' as const,
    contractVersion: 1 as const,
    type: 'run_started' as const,
    workflowId: presentation.workflow.id,
    triggerId: presentation.trigger.id,
    triggerHandler: 'trigger.manual',
    occurrenceId: 'occurrence_performance',
    runId: presentation.runId,
    occurredAt: presentation.startedAt ?? presentation.admittedAt,
  };
  const terminal = {
    contract: 'woml.trigger-progress' as const,
    contractVersion: 1 as const,
    type: 'run_terminal' as const,
    workflowId: presentation.workflow.id,
    runId: presentation.runId,
    status: 'succeeded' as const,
    occurredAt: presentation.completedAt ?? presentation.admittedAt,
  };
  const journey = (
    format: 'plain' | 'tty' | 'json',
    color: 'never' | 'always',
    verbose: boolean
  ): number => {
    let output = '';
    const foreground = new ForegroundPresentation({
      io: {
        stdout: text => { output += text; },
        stderr: text => { output += text; },
      },
      render: { ...common, format, color },
      verbose,
      inspectRun: runId => inspectRunPresentationWithRust(
        statePath,
        runId,
        { nativeCorePath }
      ),
    });
    foreground.startup(presentation.workflow);
    foreground.trigger(accepted);
    foreground.trigger(started);
    foreground.trigger(terminal);
    return textEncoder.encode(output).byteLength;
  };
  const result = {
    profile: 'woml.presentation-mode-profile/v1',
    fixture: 'woml/tests/fixtures/performance-eight-step-manual.woml',
    batches: 20,
    iterationsPerBatch: 200,
    renderModes: {
      normal: measure(() => renderRunPresentation(presentation, {
        ...common,
        format: 'plain',
        color: 'never',
      })),
      color: measure(() => renderRunPresentation(presentation, {
        ...common,
        format: 'tty',
        color: 'always',
      })),
      json: measure(() => renderRunPresentation(presentation, {
        ...common,
        format: 'json',
        color: 'never',
      })),
      verbose: measure(() => {
        const rendered = renderRunPresentation(presentation, {
          ...common,
          format: 'tty',
          color: 'always',
        });
        return `[woml:verbose] Run ${presentation.runId} completed.\n${rendered}`;
      }),
    },
    foregroundJourneyModes: {
      normal: measureJourney(() => journey('plain', 'never', false)),
      color: measureJourney(() => journey('tty', 'always', false)),
      json: measureJourney(() => journey('json', 'never', false)),
      verbose: measureJourney(() => journey('tty', 'always', true)),
    },
  };
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
} finally {
  await rm(temporaryRoot, { recursive: true, force: true });
}
