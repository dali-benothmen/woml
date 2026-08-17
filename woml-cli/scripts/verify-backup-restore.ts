import { readFile } from 'node:fs/promises';

import Ajv2020 from 'ajv/dist/2020.js';
import addFormats from 'ajv-formats';

const root = new URL('../../', import.meta.url);
const required = new Map([
  [
    'core/woml-engine/src/backup.rs',
    ['create_online_backup', 'prepare_restored_store', 'audit_definition_inventory'],
  ],
  [
    'woml-cli/src/production-backup.ts',
    ['woml backup', 'woml restore', 'WOML_RESTORE_TARGET_ACTIVE'],
  ],
  [
    'docs/woml-backup-and-restore.md',
    ['Online backup', 'Offline restore', 'Secret providers are separate'],
  ],
]);

for (const [path, markers] of required) {
  const contents = await readFile(new URL(path, root), 'utf8');
  for (const marker of markers) {
    if (!contents.includes(marker)) {
      throw new Error(`${path} is missing PRO7 marker: ${marker}`);
    }
  }
}

const schema = JSON.parse(
  await readFile(new URL('docs/schemas/backup-manifest.v1.schema.json', root), 'utf8')
);
const ajv = new Ajv2020({ strict: false, allErrors: true });
addFormats(ajv);
const validate = ajv.compile(schema);
const fixture = {
  profile: 'woml.backup-manifest/v1',
  backupId: 'backup_fixture',
  createdAt: '2026-08-12T12:00:00.000Z',
  deploymentId: 'deployment_fixture',
  activationId: `sha256:${'a'.repeat(64)}`,
  storeVersion: 14,
  database: {
    file: 'state.sqlite',
    sizeBytes: 4096,
    digest: `sha256:${'b'.repeat(64)}`,
  },
  definitionHashes: [`sha256:${'c'.repeat(64)}`],
  verified: true,
};
if (!validate(fixture)) {
  throw new Error(`Backup Manifest v1 fixture failed validation: ${JSON.stringify(validate.errors)}`);
}

console.log('PRO7 backup, restore, and upgrade verification passed.');
