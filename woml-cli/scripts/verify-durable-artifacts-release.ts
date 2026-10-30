import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { readRustModuleSource } from './rust-source';

const repositoryRoot = resolve(import.meta.dir, '../..');
const required = [
  'docs/woml-durable-artifacts.md',
  'docs/durable-artifacts-v1-threat-model.md',
  'docs/protocols/durable-artifacts-v1.md',
  'docs/schemas/artifact-inspection.v1.schema.json',
  'docs/schemas/artifact-run-inspection.v1.schema.json',
  'docs/schemas/run-inspection.v7.schema.json',
  'docs/schemas/artifact-export.v1.schema.json',
  'docs/schemas/artifact-observation.v1.schema.json',
  'docs/schemas/runtime-metrics.v2.schema.json',
  'examples/durableArtifacts/createReport.woml',
  'examples/durableArtifacts/terraformPlan.woml',
  'examples/durableArtifacts/testReport.woml',
  'examples/durableArtifacts/databaseBackup.woml',
  'examples/durableArtifacts/diagnosticBundle.woml',
  'examples/durableArtifacts/performance-budgets.v1.json',
];

const sources = new Map<string, string>();
for (const relative of required) {
  const source = await readFile(resolve(repositoryRoot, relative), 'utf8');
  if (source.trim().length === 0) throw new Error(`${relative} is empty.`);
  sources.set(relative, source);
}

const budgets = JSON.parse(
  sources.get('examples/durableArtifacts/performance-budgets.v1.json')!
) as Record<string, unknown>;
if (
  budgets.profile !== 'woml.durable-artifacts-performance-budgets/v1' ||
  Object.entries(budgets).some(([name, value]) =>
    name !== 'profile' && (!Number.isFinite(value) || Number(value) <= 0)
  )
) {
  throw new Error('Durable Artifacts performance budgets are invalid or unversioned.');
}

for (const schemaName of [
  'artifact-inspection.v1.schema.json',
  'artifact-export.v1.schema.json',
  'artifact-observation.v1.schema.json',
  'run-inspection.v7.schema.json',
]) {
  const schema = JSON.parse(
    sources.get(`docs/schemas/${schemaName}`)!
  ) as { readonly 'x-status'?: string; readonly 'x-forbiddenFields'?: readonly string[] };
  if (
    schema['x-status'] !== 'durable-artifacts-da9-frozen' ||
    !schema['x-forbiddenFields']?.includes('body') ||
    !schema['x-forbiddenFields']?.includes('credentials')
  ) {
    throw new Error(`${schemaName} does not freeze the DA9 body-exclusion boundary.`);
  }
}

const implementation = await Promise.all([
  readRustModuleSource(resolve(repositoryRoot, 'core/woml-engine/src/artifacts.rs')),
  readFile(resolve(repositoryRoot, 'woml-cli/src/runtime-observability.ts'), 'utf8'),
  readFile(resolve(repositoryRoot, 'woml-cli/src/cli.ts'), 'utf8'),
]);
const joined = implementation.join('\n');
for (const marker of [
  'woml.artifact-inspection/v1',
  'woml.artifact-export/v1',
  'woml.artifact-observation/v1',
  'woml_artifact_backend_healthy',
  'woml_artifact_operator_audit',
  'WOML_ARTIFACT_EXPORT_EXISTS',
]) {
  if (!joined.includes(marker)) throw new Error(`DA9 release marker is missing: ${marker}`);
}

const telemetrySource = implementation[1]!;
for (const forbidden of [
  'signedUrl', 'artifact.path', 'artifact.sha256', 'artifact.name',
  'resolvedSecrets', 'accessKey', 'secretKey',
]) {
  if (telemetrySource.includes(forbidden)) {
    throw new Error(`Artifact telemetry contains forbidden source marker: ${forbidden}`);
  }
}

console.log(
  `Durable Artifacts release gate passed: ${required.length} public artifacts, ` +
  'versioned performance budgets, audited export, and body-free telemetry verified.'
);
