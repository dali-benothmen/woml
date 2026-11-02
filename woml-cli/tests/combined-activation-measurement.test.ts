import { describe, expect, test } from 'bun:test';
import {
  ACTIVATION_SAMPLES,
  ACTIVATION_WARMUPS,
  measureWarmActivation,
} from '../scripts/combined-activation-measurement';

describe('Combined activation percentile measurement', () => {
  test('uses a fixed sequential cohort and keeps warm-ups separate', async () => {
    let calls = 0;
    let active = 0;
    const result = await measureWarmActivation(async () => {
      active += 1;
      expect(active).toBe(1);
      await Promise.resolve();
      active -= 1;
      calls += 1;
      return calls <= ACTIVATION_WARMUPS ? 5000 : 600;
    });
    expect(ACTIVATION_WARMUPS).toBe(3);
    expect(ACTIVATION_SAMPLES).toBe(40);
    expect(calls).toBe(43);
    expect(result.warmups).toEqual([5000, 5000, 5000]);
    expect(result.samples).toHaveLength(40);
    expect(result.summary).toMatchObject({ count: 40, p95Ms: 600, maximumMs: 600 });
  });

  test('retains a scheduling outlier without treating it as the 95th percentile', async () => {
    let calls = 0;
    const result = await measureWarmActivation(async () => {
      calls += 1;
      return calls === ACTIVATION_WARMUPS + 1 ? 1001.13 : 900;
    });
    expect(result.samples[0]).toBe(1001.13);
    expect(result.summary.maximumMs).toBe(1001.13);
    expect(result.summary.p95Ms).toBe(900);
    expect(calls).toBe(43);
  });

  test('still exposes regressions affecting more than five percent of measured runs', async () => {
    let calls = 0;
    const result = await measureWarmActivation(async () => {
      calls += 1;
      const index = calls - ACTIVATION_WARMUPS;
      return index > 0 && index <= 3 ? 1001.13 : 900;
    });
    expect(result.summary.p95Ms).toBe(1001.13);
    expect(result.summary.p95Ms).toBeGreaterThan(1000);
    expect(calls).toBe(43);
  });

  test('does not retry or replace a consistently slow cohort', async () => {
    let calls = 0;
    const result = await measureWarmActivation(async () => {
      calls += 1;
      return 1200;
    });
    expect(result.summary.p95Ms).toBe(1200);
    expect(result.samples.every(value => value === 1200)).toBe(true);
    expect(calls).toBe(43);
  });

  test('propagates execution failures without sampling again', async () => {
    let calls = 0;
    const failure = new Error('workflow failed');
    await expect(measureWarmActivation(async () => {
      calls += 1;
      if (calls === ACTIVATION_WARMUPS + 2) throw failure;
      return 600;
    })).rejects.toBe(failure);
    expect(calls).toBe(5);
  });

  test('rejects invalid warm-up or measured timings rather than omitting them', async () => {
    for (const invalid of [NaN, Infinity, -1]) {
      for (const failAt of [1, ACTIVATION_WARMUPS + 1]) {
        let calls = 0;
        await expect(measureWarmActivation(async () => {
          calls += 1;
          return calls === failAt ? invalid : 600;
        })).rejects.toThrow('finite non-negative');
        expect(calls).toBe(failAt);
      }
    }
  });
});
