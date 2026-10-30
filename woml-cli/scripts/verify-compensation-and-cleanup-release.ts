#!/usr/bin/env bun

import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { readRustModuleSource } from './rust-source';

const root = resolve(import.meta.dir, '../..');
const read = (path: string): Promise<string> =>
  path.endsWith('.rs') ? readRustModuleSource(resolve(root, path)) : readFile(resolve(root, path), 'utf8');

const required = [
  'docs/woml-compensation-and-cleanup.md',
  'docs/woml-compensation-threat-model.md',
  'docs/woml-compensation-operations.md',
  'docs/protocols/compensation-and-cleanup-v1.md',
  'docs/schemas/compiled-workflow-model.v19.schema.json',
  'docs/schemas/run-event.v18.schema.json',
  'docs/schemas/durable-state-store.v17.schema.json',
  'docs/schemas/script-host-protocol.v11.schema.json',
  'docs/schemas/correction-inspection.v1.schema.json',
  'docs/schemas/correction-recovery.v1.schema.json',
  'docs/schemas/correction-observation.v1.schema.json',
  'docs/schemas/run-inspection.v10.schema.json',
  'examples/compensationAndCleanup/README.md',
  'examples/compensationAndCleanup/previewEnvironment.woml',
  'examples/compensationAndCleanup/deploymentRollback.woml',
  'examples/compensationAndCleanup/inventoryRelease.woml',
  'examples/compensationAndCleanup/paymentRefund.woml',
  'examples/compensationAndCleanup/accessRevocation.woml',
  'examples/compensationAndCleanup/temporaryResourceCleanup.woml',
  'examples/compensationAndCleanup/performance-budgets.v1.json',
  'skills/woml/references/tags.md',
  'skills/woml/references/context-and-scripts.md',
  'skills/woml/references/cli.md',
  'skills/woml/references/diagnostics.md',
  'skills/woml/references/patterns.md',
  'woml-vscode/snippets/woml.code-snippets',
  'woml-cli/tests/compensation-and-cleanup-packaged-release.test.ts',
];

const sources = new Map<string, string>();
for (const path of required) {
  const source = await read(path);
  if (source.trim().length === 0) throw new Error(`${path} is empty.`);
  sources.set(path, source);
}

const contract = JSON.parse(
  await read('woml/tests/fixtures/compensation-and-cleanup/contracts.v1.json')
) as {
  readonly profile?: string;
  readonly modelVersion?: number;
  readonly eventVersion?: number;
  readonly storeVersion?: number;
  readonly scriptHostVersion?: number;
  readonly limits?: Record<string, number>;
};
if (
  contract.profile !== 'woml.compensation-and-cleanup/v1' ||
  contract.modelVersion !== 19 ||
  contract.eventVersion !== 18 ||
  contract.storeVersion !== 17 ||
  contract.scriptHostVersion !== 11 ||
  contract.limits?.recoveryBatchSize !== 100 ||
  contract.limits.inspectionResponseBytes !== 1_048_576
) {
  throw new Error('The frozen Compensation and Cleanup v1 contract drifted.');
}

const budgets = JSON.parse(
  sources.get('examples/compensationAndCleanup/performance-budgets.v1.json')!
) as Record<string, unknown>;
if (
  budgets.profile !== 'woml.compensation-and-cleanup-performance-budgets/v1' ||
  Object.entries(budgets).some(([key, value]) =>
    key !== 'profile' && (!Number.isFinite(value) || Number(value) <= 0)
  )
) {
  throw new Error('Compensation and Cleanup performance budgets are invalid.');
}

const implementation = (await Promise.all([
  read('core/woml-engine/src/durable.rs'),
  read('core/woml-engine/src/runtime.rs'),
  read('core/woml-engine/src/retention.rs'),
  read('core/woml-native/src/bridge.rs'),
  read('woml-cli/src/cli.ts'),
])).join('\n');
for (const marker of [
  'CORRECTION_RECOVERY_BATCH_LIMIT: usize = 100',
  'CORRECTION_UNRESOLVED_STORE_LIMIT: usize = 10_000',
  'resume_corrections_for_workflows_durable',
  'woml.correction-inspection/v1',
  'woml.correction-recovery-request/v1',
  'woml.correction-observation/v1',
  'woml recover <runId> --retry <actionId>',
  'Resolve after external repair',
]) {
  if (!implementation.includes(marker)) {
    throw new Error(`CC8 implementation marker is missing: ${marker}`);
  }
}

const snippets = sources.get('woml-vscode/snippets/woml.code-snippets')!;
if (!snippets.includes('"prefix": "woml-saga"')) {
  throw new Error('The VS Code saga snippet is missing.');
}

const skill = [
  sources.get('skills/woml/references/tags.md'),
  sources.get('skills/woml/references/context-and-scripts.md'),
  sources.get('skills/woml/references/cli.md'),
  sources.get('skills/woml/references/diagnostics.md'),
  sources.get('skills/woml/references/patterns.md'),
].join('\n');
for (const marker of [
  '<saga>',
  '<compensate>',
  '<cleanup>',
  'context.compensation',
  'context.cleanup',
  'woml recover',
  'WOML_CORRECTION_RECOVERY_REQUIRED',
]) {
  if (!skill.includes(marker)) {
    throw new Error(`WOML AI skill correction marker is missing: ${marker}`);
  }
}

const ci = await read('.github/workflows/ci.yml');
for (const marker of [
  'Compensation and cleanup',
  'ubuntu-24.04',
  'macos-15',
  'windows-2022',
  'compensation_runtime',
  'compensation_store',
  'compensation-and-cleanup-packaged-release.test.ts',
]) {
  if (!ci.includes(marker)) {
    throw new Error(`CC8 CI release gate is missing: ${marker}`);
  }
}

console.log(
  `Compensation and Cleanup release gate passed: ${required.length} public artifacts, ` +
  'versioned contracts, examples, editor, AI skill, recovery, package, and cross-platform CI verified.'
);
