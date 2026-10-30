#!/usr/bin/env bun

import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { readRustModuleSource } from './rust-source';

const root = resolve(import.meta.dir, '../..');
const read = (path: string): Promise<string> =>
  path.endsWith('.rs') ? readRustModuleSource(resolve(root, path)) : readFile(resolve(root, path), 'utf8');

const required = [
  'docs/infrastructure-policies-v1.md',
  'docs/release/infrastructure-policies-v1-release-notes.md',
  'docs/schemas/infrastructure-policy.v1.schema.json',
  'docs/schemas/infrastructure-policy-decision.v1.schema.json',
  'docs/schemas/infrastructure-policy-run-inspection.v1.schema.json',
  'woml/tests/fixtures/infrastructure-policies/coverage.v1.json',
  'woml/tests/fixtures/infrastructure-policies/threat-model.v1.json',
  'examples/infrastructurePolicies/README.md',
  'examples/infrastructurePolicies/reviewedTerraformRelease.woml',
  'examples/infrastructurePolicies/production-policy.example.json',
  'examples/infrastructurePolicies/emergency-deny.example.json',
  'examples/infrastructurePolicies/process-policy.example.json',
  'examples/infrastructurePolicies/artifact-policy.example.json',
  'examples/infrastructurePolicies/reviewer-directory.example.json',
  'examples/infrastructurePolicies/performance-budgets.v1.json',
  'skills/woml/SKILL.md',
  'skills/woml/references/cli.md',
  'woml-cli/tests/infrastructure-policy-packaged-release.test.ts',
];

const sources = new Map<string, string>();
for (const path of required) {
  const source = await read(path);
  if (source.trim().length === 0) throw new Error(`${path} is empty.`);
  sources.set(path, source);
}

const coverage = JSON.parse(
  sources.get('woml/tests/fixtures/infrastructure-policies/coverage.v1.json')!
) as {
  readonly profile?: string;
  readonly paths?: readonly { readonly path?: string; readonly posture?: string }[];
};
if (
  coverage.profile !== 'woml.infrastructure-policy-coverage/v1' ||
  coverage.paths === undefined ||
  coverage.paths.length < 16 ||
  new Set(coverage.paths.map(item => item.path)).size !== coverage.paths.length
) {
  throw new Error('The IP0 protected-effect inventory is incomplete or ambiguous.');
}

const implementation = (await Promise.all([
  read('core/woml-engine/src/capability.rs'),
  read('core/woml-engine/src/durable.rs'),
  read('core/woml-engine/src/event.rs'),
  read('core/woml-engine/src/infrastructure_policy.rs'),
  read('core/woml-engine/src/infrastructure_policy_runtime.rs'),
  read('core/woml-engine/src/runtime.rs'),
  read('core/woml-native/src/bridge.rs'),
  read('woml-cli/src/cli.ts'),
  read('woml-cli/src/infrastructure-policy.ts'),
])).join('\n');
for (const marker of [
  'infrastructure_policy_descriptor_coverage',
  'InfrastructurePolicyDecisionRecorded',
  'WOML_POLICY_INPUT_UNSUPPORTED',
  'WOML_POLICY_REQUIREMENTS_UNSATISFIED',
  'WOML_POLICY_REQUIRED',
  'prepare_emergency_deny_overlay',
  'verify_policy_evidence',
  'woml_infrastructure_policy_bundles_no_update',
  'woml_infrastructure_policy_bundles_no_delete',
  'INFRASTRUCTURE_POLICY_MAX_BYTES: usize = 1024 * 1024',
  'INFRASTRUCTURE_POLICY_MAX_RULES: usize = 1024',
  'DEFAULT_MAX_EVALUATION_INSTRUCTIONS: u64 = 250_000',
  'Duration::from_millis(100)',
]) {
  if (!implementation.includes(marker)) {
    throw new Error(`IP8 authority or bypass marker is missing: ${marker}`);
  }
}

const evaluator = await read(
  'core/woml-engine/src/infrastructure_policy_runtime.rs'
);
for (const forbidden of ['reqwest::Client', 'Bun.spawn', 'script-host-worker']) {
  if (evaluator.includes(forbidden)) {
    throw new Error(`Policy evaluation acquired a forbidden round trip: ${forbidden}`);
  }
}

const budgets = JSON.parse(
  sources.get('examples/infrastructurePolicies/performance-budgets.v1.json')!
) as {
  readonly profile?: string;
  readonly authority?: string;
  readonly budgets?: Readonly<Record<string, number>>;
};
if (
  budgets.profile !== 'woml.infrastructure-policy-performance-budgets/v1' ||
  budgets.authority !== 'in-process-rust' ||
  budgets.budgets?.maximumPolicyBytes !== 1_048_576 ||
  budgets.budgets.maximumRules !== 1_024 ||
  budgets.budgets.maximumEvaluationInstructions !== 250_000 ||
  budgets.budgets.maximumPolicyEvaluationMicros !== 100_000 ||
  budgets.budgets.maximumDecisionBytes !== 65_536 ||
  Object.values(budgets.budgets).some(value => !Number.isFinite(value) || value <= 0)
) {
  throw new Error('Infrastructure Policy publication budgets are invalid.');
}

for (const schemaName of [
  'infrastructure-policy.v1.schema.json',
  'infrastructure-policy-decision.v1.schema.json',
  'infrastructure-policy-run-inspection.v1.schema.json',
]) {
  const schema = JSON.parse(
    sources.get(`docs/schemas/${schemaName}`)!
  ) as { readonly additionalProperties?: boolean };
  if (schema.additionalProperties !== false) {
    throw new Error(`${schemaName} is not fail-closed at its public root.`);
  }
}

const example = sources.get(
  'examples/infrastructurePolicies/reviewedTerraformRelease.woml'
)!;
for (const marker of [
  'services.process.run(',
  'services.artifacts.put(',
  '<approval',
  'approvals="2"',
  '<lock',
  '<saga',
  '<compensate',
  '<cleanup>',
]) {
  if (!example.includes(marker)) {
    throw new Error(`The reviewed Terraform example is missing ${marker}.`);
  }
}
if (example.includes('Bun.spawn(')) {
  throw new Error('The production example bypasses the supervised process authority.');
}

const publicGuidance = [
  sources.get('docs/infrastructure-policies-v1.md'),
  sources.get('skills/woml/references/cli.md'),
].join('\n');
for (const marker of [
  'advisory',
  'final decision always belongs to the Rust authority',
  'cannot prove',
  '--require-policy-signature',
  '--emergency-deny',
]) {
  if (!publicGuidance.toLowerCase().includes(marker.toLowerCase())) {
    throw new Error(`IP8 public security guidance is missing: ${marker}`);
  }
}

const ci = await read('.github/workflows/ci.yml');
for (const marker of [
  'Infrastructure Policies (${{ matrix.os }})',
  'ubuntu-24.04',
  'macos-15',
  'windows-2022',
  'infrastructure_policy_contract',
  'infrastructure_policy_authority',
  'verify-infrastructure-policy-release.ts',
  'infrastructure-policy-packaged-release.test.ts',
]) {
  if (!ci.includes(marker)) throw new Error(`IP8 CI release gate is missing: ${marker}`);
}

const release = await read('.github/workflows/release.yml');
if (!release.includes('verify-infrastructure-policy-release.ts')) {
  throw new Error('The release workflow does not enforce the IP8 source gate.');
}

console.log(
  `Infrastructure Policies release gate passed: ${required.length} public artifacts, ` +
    `${coverage.paths.length} effect paths, frozen budgets, packaged CLI, and cross-platform authority CI verified.`
);
