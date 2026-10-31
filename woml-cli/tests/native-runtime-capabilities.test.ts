import { expect, test } from 'bun:test';
import { encodeNativeRuntimeCapabilities } from '../src/native-runtime-capabilities';
import type { RuntimeCapabilitiesV1 } from '../src/rust-executor';

test('policy and emergency authority use native wire keys without changing JS options', () => {
  const base = {
    policy: {
      contract: 'woml.infrastructure-policy', contractVersion: 1,
      id: 'wire-test', version: '1', environment: 'test',
      defaults: { read: 'deny', 'idempotent-write': 'deny', 'unsafe-write': 'deny' },
      rules: [],
    },
    trust: {}, required: true,
  } as const;
  const input: RuntimeCapabilitiesV1 = {
    infrastructurePolicy: base,
    emergencyDeny: { ...base, required: false },
  };
  const before = JSON.stringify(input);
  const decoded = JSON.parse(encodeNativeRuntimeCapabilities(input));
  expect(Object.keys(decoded).sort()).toEqual(['emergency_deny', 'infrastructure_policy']);
  expect(decoded.infrastructure_policy).toEqual(input.infrastructurePolicy);
  expect(decoded.emergency_deny).toEqual(input.emergencyDeny);
  expect(JSON.stringify(input)).toBe(before);
});

test('empty and reviewer-only authority retain their existing native shapes', () => {
  expect(encodeNativeRuntimeCapabilities({})).toBe('{}');
  const input = { reviewers: { directory: { profile: 'woml.reviewer-directory/v1', version: '1', principals: [] } } } as const;
  expect(JSON.parse(encodeNativeRuntimeCapabilities(input))).toEqual(input);
});
