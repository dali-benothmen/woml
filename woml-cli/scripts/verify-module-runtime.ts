import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import { buildWomlRuntimeDefinitionPackage, parseWoml } from 'woml';

const repositoryRoot = resolve(import.meta.dir, '../..');
const fixtureRoot = resolve(repositoryRoot, 'woml/tests/fixtures/modules');
const workflowPath = resolve(fixtureRoot, 'customer-import.woml');
for (const file of [
  'docs/protocols/module-runtime-v1.md',
  'docs/schemas/script-host-protocol.v5.schema.json',
  'docs/schemas/woml-definition-package.v3.schema.json',
  'examples/moduleWorkflow.woml',
  'examples/modules/spreadsheet.ts',
  'woml/tests/fixtures/modules/customer-import.package.v3.identity.json',
]) {
  readFileSync(resolve(repositoryRoot, file));
}

const source = readFileSync(workflowPath, 'utf8');
const definitionPackage = await buildWomlRuntimeDefinitionPackage(
  parseWoml(source, { file: workflowPath }),
  { sourcePath: workflowPath, projectRoot: fixtureRoot }
);
if (
  definitionPackage.rootHash !==
    'sha256:b487c0460824b7730fd6bd5b3b07c9047a92116dffbb3fe443749c5d9bb74daf' ||
  definitionPackage.schemaVersion !== 3 ||
  definitionPackage.runtimeReady !== true
) {
  throw new Error('MS3 runtime package identity drifted.');
}
const serialized = JSON.stringify(definitionPackage);
for (const forbidden of [repositoryRoot, '/tmp/', 'SLACK_BOT_TOKEN', 'xoxb-']) {
  if (serialized.includes(forbidden)) {
    throw new Error(`MS3 runtime package contains forbidden material: ${forbidden}`);
  }
}

console.log(
  '[MS3] Package v3, Model v9, Script Host v5, isolated module execution, native Fetch, and managed services are publishable'
);
