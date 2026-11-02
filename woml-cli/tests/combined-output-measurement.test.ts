import { describe, expect, test } from 'bun:test';
import {
  OUTPUT_SAMPLE_PAIRS,
  OUTPUT_WARMUP_PAIRS,
  measureStreamedOutput,
} from '../scripts/combined-output-measurement';

describe('Combined streamed-output measurement', () => {
  test('uses fixed sequential pairs with balanced order and separate warm-ups', async () => {
    let active = 0;
    const calls: string[] = [];
    const result = await measureStreamedOutput(async (variant, run) => {
      expect(++active).toBe(1);
      calls.push(`${run.phase}-${run.index}-${variant}`);
      await Promise.resolve();
      active -= 1;
      return run.phase === 'warmup' ? 5000 : variant === 'quiet' ? 400 : 600;
    });
    expect(OUTPUT_WARMUP_PAIRS).toBe(2);
    expect(OUTPUT_SAMPLE_PAIRS).toBe(12);
    expect(calls).toHaveLength(28);
    expect(new Set(calls).size).toBe(28);
    expect(calls.slice(0, 8)).toEqual([
      'warmup-0-quiet', 'warmup-0-noisy', 'warmup-1-noisy', 'warmup-1-quiet',
      'measured-0-quiet', 'measured-0-noisy', 'measured-1-noisy', 'measured-1-quiet',
    ]);
    expect(result.warmups).toHaveLength(2);
    expect(result.warmups[0]!.quietMilliseconds).toBe(5000);
    expect(result.pairs.filter(pair => pair.order[0] === 'quiet')).toHaveLength(6);
    expect(result.pairs.filter(pair => pair.order[0] === 'noisy')).toHaveLength(6);
    expect(result.quietSummary).toMatchObject({ count: 12, medianMs: 400 });
    expect(result.noisySummary).toMatchObject({ count: 12, medianMs: 600 });
    expect(result.overheadPercent).toBe(50);
  });

  test('uses both central values for each median and keeps the ratio-of-medians metric', async () => {
    const result = await measureStreamedOutput(async (variant, { index }) => {
      // Medians: quiet = (100 + 300) / 2; noisy = (400 + 600) / 2.
      if (variant === 'quiet') return index < 6 ? 100 : 300;
      return index < 6 ? 600 : 400;
    });
    expect(result.quietSummary.medianMs).toBe(200);
    expect(result.noisySummary.medianMs).toBe(500);
    expect(result.overheadPercent).toBe(150);
  });

  test('retains slow measured runs in raw timings and summaries', async () => {
    const result = await measureStreamedOutput(async (variant, { phase, index }) =>
      phase === 'measured' && variant === 'noisy' && index === 0
        ? 10_000 : variant === 'quiet' ? 400 : 600
    );
    expect(result.pairs[0]!.noisyMilliseconds).toBe(10_000);
    expect(result.noisySummary.maximumMs).toBe(10_000);
    expect(result.noisySummary.count).toBe(12);
    expect(result.overheadPercent).toBe(50);
  });

  test('still fails the frozen 200 percent limit for sustained 213.67 percent overhead', async () => {
    let calls = 0;
    const result = await measureStreamedOutput(async variant => {
      calls += 1;
      return variant === 'quiet' ? 1000 : 3136.7;
    });
    expect(result.overheadPercent).toBeCloseTo(213.67, 8);
    expect(result.overheadPercent).toBeGreaterThan(200);
    expect(calls).toBe(28);
  });

  test('does not retry execution failures in warm-up or measured work', async () => {
    for (const failAt of [1, OUTPUT_WARMUP_PAIRS * 2 + 2]) {
      let calls = 0;
      const failure = new Error('workflow failed');
      await expect(measureStreamedOutput(async () => {
        if (++calls === failAt) throw failure;
        return 400;
      })).rejects.toBe(failure);
      expect(calls).toBe(failAt);
    }
  });

  test('rejects invalid timings without omitting or replacing them', async () => {
    for (const invalid of [NaN, Infinity, -1, 0]) {
      for (const failAt of [1, OUTPUT_WARMUP_PAIRS * 2 + 1]) {
        let calls = 0;
        await expect(measureStreamedOutput(async () =>
          ++calls === failAt ? invalid : 400
        )).rejects.toThrow('finite positive');
        expect(calls).toBe(failAt);
      }
    }
  });

  test('reports zero overhead rather than a negative percentage when noisy is faster', async () => {
    const result = await measureStreamedOutput(async variant => variant === 'quiet' ? 400 : 300);
    expect(result.overheadPercent).toBe(0);
  });
});
