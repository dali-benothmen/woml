import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import {
  buildWomlExecutableDefinitionPackage,
  parseWoml,
} from 'woml';

const repositoryRoot = resolve(import.meta.dir, '../..');
const fixtureRoot = resolve(repositoryRoot, 'woml/tests/fixtures/modules');
const workflowPath = resolve(fixtureRoot, 'customer-import.woml');
const requiredFiles = [
  'docs/protocols/module-compilation-v1.md',
  'docs/schemas/compiled-workflow-model.v9.schema.json',
  'docs/schemas/woml-definition-package.v2.schema.json',
  'woml/tests/fixtures/modules/customer-import.compiled.v9.json',
  'woml/tests/fixtures/modules/spreadsheet.bundle.v1.mjs',
  'woml/tests/fixtures/modules/spreadsheet.bundle.v1.mjs.map',
  'woml/tests/fixtures/modules/services.generated.v1.d.ts',
];

for (const file of requiredFiles) readFileSync(resolve(repositoryRoot, file));

const source = readFileSync(workflowPath, 'utf8');
const definitionPackage = await buildWomlExecutableDefinitionPackage(
  parseWoml(source, { file: workflowPath }),
  { sourcePath: workflowPath, projectRoot: fixtureRoot }
);

if (
  definitionPackage.rootHash !==
  'sha256:3d715e5baf1fc58e050d4bec9a884129c77e5bbe7fa4341cef4416204761c9fc'
) {
  throw new Error('MS2 Definition Package v2 identity drifted.');
}
if (
  definitionPackage.workflow.model.schemaVersion !== 9 ||
  definitionPackage.runtimeReady !== false ||
  definitionPackage.artifacts.length !== 4
) {
  throw new Error('MS2 package/model/runtime gate does not match the frozen profile.');
}
const serialized = JSON.stringify(definitionPackage);
for (const forbidden of [repositoryRoot, '/tmp/', 'SLACK_BOT_TOKEN', 'xoxb-']) {
  if (serialized.includes(forbidden)) {
    throw new Error(`MS2 artifact contains forbidden material: ${forbidden}`);
  }
}

console.log(
  '[MS2] deterministic ESM bundle, source map, declarations, Model v9, and Definition Package v2 remain pinned'
);
