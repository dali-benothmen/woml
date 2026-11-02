import { summarizeSamples } from './performance-measurement';

// With five samples, nearest-rank p95 is the maximum. A fixed forty-run
// cohort estimates the tail without making one scheduling pause the p95.
// Never retry a failed cohort, discard measured outliers, or select a best run.
export const ACTIVATION_WARMUPS = 3;
export const ACTIVATION_SAMPLES = 40;

export async function measureWarmActivation(sample: () => Promise<number>) {
  const take = async (): Promise<number> => {
    const value = await sample();
    if (!Number.isFinite(value) || value < 0) {
      throw new Error('Activation timings must be finite non-negative values.');
    }
    return value;
  };
  const warmups: number[] = [];
  for (let index = 0; index < ACTIVATION_WARMUPS; index += 1) {
    warmups.push(await take());
  }
  const samples: number[] = [];
  for (let index = 0; index < ACTIVATION_SAMPLES; index += 1) {
    samples.push(await take());
  }
  return {
    warmups,
    samples,
    summary: summarizeSamples(samples),
  };
}
