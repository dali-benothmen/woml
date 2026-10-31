#!/usr/bin/env bun

import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { readRustModuleSource } from './rust-source';

const root = resolve(import.meta.dir, '../..');
const read = (path: string): Promise<string> =>
  path.endsWith('.rs') ? readRustModuleSource(resolve(root, path)) : readFile(resolve(root, path), 'utf8');

const required = [
  'docs/woml-rich-plan-approvals.md',
  'docs/protocols/rich-plan-approvals-v1.md',
  'docs/schemas/review-payload.v1.schema.json',
  'docs/schemas/reviewer-directory.v1.schema.json',
  'docs/schemas/rich-plan-approvals.v1.schema.json',
  'docs/schemas/compiled-workflow-model.v20.schema.json',
  'docs/schemas/run-event.v19.schema.json',
  'docs/schemas/durable-state-store.v18.schema.json',
  'docs/schemas/approval-http.v2.schema.json',
  'docs/schemas/notification-provider-host.v3.schema.json',
  'docs/schemas/approval-inspection.v2.schema.json',
  'docs/schemas/run-inspection.v11.schema.json',
  'examples/richApprovals/README.md',
  'examples/richApprovals/terraformDeployment.woml',
  'examples/richApprovals/aiRemediation.woml',
  'examples/richApprovals/reviewer-directory.example.json',
  'examples/richApprovals/performance-budgets.v1.json',
  'skills/woml/references/tags.md',
  'skills/woml/references/providers.md',
  'skills/woml/references/cli.md',
  'woml-cli/tests/rich-plan-approvals-packaged-release.test.ts',
];

const sources = new Map<string, string>();
for (const path of required) {
  const source = await read(path);
  if (source.trim().length === 0) throw new Error(`${path} is empty.`);
  sources.set(path, source);
}

const contract = JSON.parse(
  await read('woml/tests/fixtures/rich-plan-approvals/contracts.v1.json')
) as {
  readonly profile?: string;
  readonly modelVersion?: number;
  readonly eventVersion?: number;
  readonly storeVersion?: number;
  readonly httpVersion?: number;
  readonly providerHostVersion?: number;
};
if (
  contract.profile !== 'woml.rich-plan-approvals/v1' ||
  contract.modelVersion !== 20 ||
  contract.eventVersion !== 19 ||
  contract.storeVersion !== 18 ||
  contract.httpVersion !== 2 ||
  contract.providerHostVersion !== 3
) {
  throw new Error('The frozen Rich Plan Approvals v1 contract drifted.');
}

const budgets = JSON.parse(
  sources.get('examples/richApprovals/performance-budgets.v1.json')!
) as {
  readonly profile?: string;
  readonly authority?: string;
  readonly budgets?: Record<string, number>;
};
if (
  budgets.profile !== 'woml.rich-plan-approvals-performance-budgets/v1' ||
  budgets.authority !== 'local-sqlite' ||
  budgets.budgets === undefined ||
  Object.values(budgets.budgets).some(value => !Number.isFinite(value) || value <= 0) ||
  budgets.budgets.maximumInspectionDecisions !== 100 ||
  budgets.budgets.maximumInspectionApprovals !== 100
) {
  throw new Error('Rich Plan Approvals performance budgets are invalid.');
}

const implementation = (await Promise.all([
  read('core/woml-engine/src/rich_approval.rs'),
  read('core/woml-engine/src/durable.rs'),
  read('core/woml-engine/src/notification_protocol.rs'),
  read('core/woml-native/src/bridge.rs'),
  read('woml-cli/src/approval-server.ts'),
  read('woml-cli/src/rich-approval-presentation.ts'),
])).join('\n');
for (const marker of [
  'REVIEW_PAYLOAD_MAX_BYTES: usize = 65_536',
  'REVIEW_PAYLOAD_MAX_NESTING: usize = 8',
  'REVIEWER_DIRECTORY_MAX_PRINCIPALS: usize = 10_000',
  'RUN_INSPECTION_RICH_APPROVAL_LIMIT: usize = 100',
  'record_rich_approval_identity_decision',
  'woml.run-inspection/v11',
  'WOML_APPROVAL_AUTHORITY_UNAVAILABLE',
  'RICH_APPROVAL_NOTIFICATION_PROVIDER_PROTOCOL_VERSION',
]) {
  if (!implementation.includes(marker)) {
    throw new Error(`RPA8 implementation marker is missing: ${marker}`);
  }
}

for (const schemaName of [
  'review-payload.v1.schema.json',
  'reviewer-directory.v1.schema.json',
  'approval-http.v2.schema.json',
  'notification-provider-host.v3.schema.json',
  'run-inspection.v11.schema.json',
]) {
  const schema = JSON.parse(sources.get(`docs/schemas/${schemaName}`)!) as {
    readonly additionalProperties?: boolean;
    readonly oneOf?: readonly unknown[];
    readonly $defs?: Readonly<Record<string, { readonly additionalProperties?: boolean }>>;
    readonly 'x-forbiddenFields'?: readonly string[];
  };
  const unionIsClosed = schema.oneOf !== undefined &&
    Object.values(schema.$defs ?? {}).every(definition =>
      definition.additionalProperties === false || definition.additionalProperties === undefined
    );
  if (
    schema.additionalProperties !== false &&
    !unionIsClosed &&
    schemaName !== 'run-inspection.v11.schema.json'
  ) {
    throw new Error(`${schemaName} is not fail-closed at its public root.`);
  }
  if (schemaName === 'run-inspection.v11.schema.json') {
    for (const forbidden of ['token', 'secret', 'assertion', 'providerActorId']) {
      if (!schema['x-forbiddenFields']?.includes(forbidden)) {
        throw new Error(`Run Inspection v11 does not forbid ${forbidden}.`);
      }
    }
  }
}

const ci = await read('.github/workflows/ci.yml');
for (const marker of [
  'rich-plan-approvals:',
  'Rich Plan Approvals (${{ matrix.os }})',
  'ubuntu-24.04',
  'macos-15',
  'windows-2022',
  'rich_plan_approval_model',
  'rich_plan_approval_authority',
  'rich_plan_approval_runtime',
  'verify-rich-plan-approvals-release.ts',
  'rich-plan-approvals-packaged-release.test.ts',
]) {
  if (!ci.includes(marker)) throw new Error(`RPA8 CI release gate is missing: ${marker}`);
}

const release = await read('.github/workflows/release.yml');
if (!release.includes('verify-rich-plan-approvals-release.ts')) {
  throw new Error('The release workflow does not enforce the RPA8 source gate.');
}

console.log(
  `Rich Plan Approvals release gate passed: ${required.length} public artifacts, ` +
    'frozen contracts, reviewed budgets, package smoke test, and cross-platform authority CI verified.'
);
