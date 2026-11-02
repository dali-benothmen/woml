import { summarizeSamples } from './performance-measurement';

export const OUTPUT_WARMUP_PAIRS = 2;
export const OUTPUT_SAMPLE_PAIRS = 12;

export type OutputVariant = 'quiet' | 'noisy';
export interface OutputMeasurementRun {
  readonly phase: 'warmup' | 'measured';
  readonly index: number;
}

interface OutputTimingPair {
  readonly order: readonly OutputVariant[];
  readonly quietMilliseconds: number;
  readonly noisyMilliseconds: number;
}

// AB/BA alternation balances which workload runs first. Both variants must
// retain identical fresh-store conditions, including output-store startup.
// Use one fixed cohort: never retry it or remove slow measured samples.
export async function measureStreamedOutput(
  sample: (variant: OutputVariant, run: OutputMeasurementRun) => Promise<number>
) {
  const takePair = async (run: OutputMeasurementRun): Promise<OutputTimingPair> => {
    const order: readonly OutputVariant[] = run.index % 2 === 0
      ? ['quiet', 'noisy'] : ['noisy', 'quiet'];
    const timings = { quiet: 0, noisy: 0 };
    for (const variant of order) {
      const value = await sample(variant, run);
      if (!Number.isFinite(value) || value <= 0) {
        throw new Error('Output timings must be finite positive values.');
      }
      timings[variant] = value;
    }
    return {
      order,
      quietMilliseconds: timings.quiet,
      noisyMilliseconds: timings.noisy,
    };
  };
  const warmups: OutputTimingPair[] = [];
  for (let index = 0; index < OUTPUT_WARMUP_PAIRS; index += 1) {
    warmups.push(await takePair({ phase: 'warmup', index }));
  }
  const pairs: OutputTimingPair[] = [];
  for (let index = 0; index < OUTPUT_SAMPLE_PAIRS; index += 1) {
    pairs.push(await takePair({ phase: 'measured', index }));
  }
  const quietSummary = summarizeSamples(pairs.map(pair => pair.quietMilliseconds));
  const noisySummary = summarizeSamples(pairs.map(pair => pair.noisyMilliseconds));
  // Preserve the gate's ratio-of-medians metric, not median-of-pair-ratios.
  // summarizeSamples averages both central values for an even-sized cohort.
  const overheadPercent = Math.max(
    0,
    ((noisySummary.medianMs - quietSummary.medianMs) / quietSummary.medianMs) * 100
  );
  return { warmups, pairs, quietSummary, noisySummary, overheadPercent };
}
