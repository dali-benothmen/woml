import { readFile } from 'node:fs/promises';

import Ajv2020 from 'ajv/dist/2020.js';
import addFormats from 'ajv-formats';

const root = new URL('../../', import.meta.url);
const required = new Map([
  [
    'core/woml-engine/src/retention.rs',
    ['RETENTION_BATCH_RUNS', 'woml_maintenance_lease', 'TRIGGER_DEDUPLICATION_SAFETY_DAYS'],
  ],
  [
    'woml-cli/src/production-retention.ts',
    ['woml prune --before <duration>', 'startAutomaticRetention', 'executeRetentionWithRustAsync'],
  ],
  [
    'docs/woml-retention-and-maintenance.md',
    ['stateEntriesDeleted: 0', '30-day safety window', 'woml_retention_total'],
  ],
]);

for (const [path, markers] of required) {
  const contents = await readFile(new URL(path, root), 'utf8');
  for (const marker of markers) {
    if (!contents.includes(marker)) {
      throw new Error(`${path} is missing PRO8 marker: ${marker}`);
    }
  }
}

const schema = JSON.parse(
  await readFile(new URL('docs/schemas/retention.v1.schema.json', root), 'utf8')
);
const ajv = new Ajv2020({ strict: false, allErrors: true });
addFormats(ajv);
const validate = ajv.compile(schema);
for (const fixture of [
  {
    profile: 'woml.retention/v1',
    kind: 'plan',
    policyId: 'retention_fixture',
    succeededBefore: '2026-07-01T00:00:00.000Z',
    failedBefore: '2026-05-01T00:00:00.000Z',
    cancelledBefore: '2026-07-01T00:00:00.000Z',
    eligibleRuns: 12,
    estimatedBytes: 8192,
  },
  {
    profile: 'woml.retention/v1',
    kind: 'result',
    policyId: 'retention_fixture',
    completedAt: '2026-08-12T03:00:01.000Z',
    deletedRuns: 12,
    deletedBytes: 8192,
    stateEntriesDeleted: 0,
  },
]) {
  if (!validate(fixture)) {
    throw new Error(
      `Retention v1 fixture failed validation: ${JSON.stringify(validate.errors)}`
    );
  }
}

console.log('PRO8 retention and storage maintenance verification passed.');
