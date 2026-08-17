import { describe, expect, test } from 'bun:test';

import {
  scanSensitiveText,
  verifyFinalReleaseReview,
} from '../scripts/verify-final-release-review';

describe('final v1 security, legal, and product review', () => {
  test('detects credential and machine-specific release content', () => {
    expect(
      scanSensitiveText(
        'fixture.txt',
        ['token=xoxb-', '12345678901234567890\npath=/home/', 'private-user/project'].join(''),
      ).map(finding => finding.kind),
    ).toEqual(['credential', 'machine-identity']);
  });

  test('accepts public examples and portable machine paths', () => {
    expect(
      scanSensitiveText(
        'fixture.txt',
        'https://example.test/callback\n/home/runner/work/woml\n127.0.0.1',
      ),
    ).toEqual([]);
  });

  test('reviews the tracked tree and the exact staged package', async () => {
    const result = await verifyFinalReleaseReview();
    expect(result.sourceFiles).toBeGreaterThan(900);
    expect(result.packageFiles).toBe(18);
    expect(result.packageDigest).toMatch(/^[a-f0-9]{64}$/u);
  });
});
