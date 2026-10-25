#!/usr/bin/env bun

import { readFile, stat } from 'node:fs/promises';
import { resolve } from 'node:path';

const root = resolve(import.meta.dir, '../..');
const read = (path: string): Promise<string> =>
  readFile(resolve(root, path), 'utf8');

const required = [
  'docs/woml-opentelemetry.md',
  'docs/protocols/opentelemetry-export-v1.md',
  'docs/opentelemetry-export-v1-threat-model.md',
  'docs/opentelemetry-export-v1-source-inventory.md',
  'docs/release/opentelemetry-export-v1-release-notes.md',
  'docs/schemas/opentelemetry-configuration.v1.schema.json',
  'docs/schemas/opentelemetry-exporter-health.v1.schema.json',
  'docs/schemas/opentelemetry-exporter-health.v2.schema.json',
  'docs/schemas/telemetry-signal.v1.schema.json',
  'docs/schemas/telemetry-admission-link.v1.schema.json',
  'examples/openTelemetry/README.md',
  'examples/openTelemetry/collector-config.yaml',
  'examples/openTelemetry/docker-compose.yml',
  'examples/openTelemetry/woml.runtime.json',
  'woml-cli/tests/opentelemetry-release.test.ts',
  'woml-cli/tests/opentelemetry-packaged-release.test.ts',
  'woml-cli/scripts/benchmark-opentelemetry.ts',
];
const sources = new Map<string, string>();
for (const path of required) {
  const source = await read(path);
  if (source.trim().length === 0) throw new Error(`${path} is empty.`);
  sources.set(path, source);
}

const contracts = JSON.parse(
  await read('woml-cli/tests/fixtures/opentelemetry/contracts.v1.json')
);
const budgets = contracts.performanceBudgets;
if (
  budgets.baseline.cliBytes !== 1_188_910 ||
  budgets.baseline.cliMeasurement !==
    'feature/combined-developer-experience@CDX8-workflow-json-clean-install' ||
  budgets.disabled.activationRegressionMsMax !== 5 ||
  budgets.disabled.shortRunRegressionMsMax !== 2 ||
  budgets.disabled.rssBytesMax !== 2_097_152 ||
  budgets.disabled.timers !== 0 ||
  budgets.disabled.sockets !== 0 ||
  budgets.packaging.mainBundleIncreaseBytesMax !== 20_480 ||
  budgets.packaging.sdkBundleBytesMax !== 2_621_440 ||
  budgets.packaging.installedIncreaseBytesMax !== 3_145_728
) {
  throw new Error('OpenTelemetry Export performance budgets drifted.');
}

const releaseArtifacts = [
  'woml-cli/dist/cli.js',
  'woml-cli/dist/opentelemetry-runtime.js',
  'woml-cli/dist/opentelemetry-sdk.js',
];
for (const path of releaseArtifacts) {
  if ((await stat(resolve(root, path))).size === 0) {
    throw new Error(`OpenTelemetry release artifact is missing or empty: ${path}`);
  }
}
const cliBundle = await read('woml-cli/dist/cli.js');
for (const marker of [
  'OpenTelemetry queue capacity was reached',
  'One or more OpenTelemetry providers failed to settle',
]) {
  if (cliBundle.includes(marker)) {
    throw new Error(`Optional OpenTelemetry implementation leaked into the main bundle: ${marker}`);
  }
}
const runtimeBundle = await read('woml-cli/dist/opentelemetry-runtime.js');
const sdkBundle = await read('woml-cli/dist/opentelemetry-sdk.js');
if (!runtimeBundle.includes('OpenTelemetry queue capacity was reached')) {
  throw new Error('The lazy OpenTelemetry runtime bundle is incomplete.');
}
if (!sdkBundle.includes('One or more OpenTelemetry providers failed to settle')) {
  throw new Error('The lazy OpenTelemetry SDK bundle is incomplete.');
}

const packageContract = await read('woml-cli/package.json');
for (const marker of [
  'dist/opentelemetry-runtime.js',
  'dist/opentelemetry-sdk.js',
  'test:opentelemetry',
  'benchmark:opentelemetry',
  'test:opentelemetry-package',
]) {
  if (!packageContract.includes(marker)) {
    throw new Error(`OpenTelemetry package/release contract is missing ${marker}.`);
  }
}

const notice = await read('NOTICE.md');
const packageNotice = await read('woml-cli/NOTICE.md');
if (notice !== packageNotice) {
  throw new Error('The source-package NOTICE.md differs from the repository notice.');
}
for (const marker of [
  '@opentelemetry/api',
  '@opentelemetry/sdk-trace',
  '@opentelemetry/sdk-metrics',
  '@opentelemetry/sdk-logs',
  'Apache License 2.0',
]) {
  if (!notice.includes(marker)) {
    throw new Error(`The dependency notice is missing ${marker}.`);
  }
}

const receiverTest = sources.get('woml-cli/tests/opentelemetry-release.test.ts')!;
for (const marker of [
  "'/v1/traces'",
  "'/v1/metrics'",
  "'/v1/logs'",
  '10_000',
  '2_048',
  'OTE8_RECEIVER_SECRET_7fd25a',
]) {
  if (!receiverTest.includes(marker)) {
    throw new Error(`The bounded local collector journey is missing ${marker}.`);
  }
}

const securitySources = [
  await read('woml-cli/src/opentelemetry-signal.ts'),
  await read('woml-cli/src/opentelemetry-observation.ts'),
  await read('woml-cli/src/opentelemetry-runtime.ts'),
  await read('woml-cli/src/opentelemetry-sdk.ts'),
].join('\n');
for (const forbidden of [
  'workflow.value',
  'artifact.content',
  'process.output',
  'database.statement',
  'exception.stack',
]) {
  if (securitySources.includes(`attributes['${forbidden}']`)) {
    throw new Error(`A denied telemetry attribute is emitted: ${forbidden}`);
  }
}

const compatibility = [
  await read('woml-cli/tests/production-runtime-contracts.test.ts'),
  await read('woml-cli/tests/protocol-contract.test.ts'),
  await read('core/woml-engine/tests/durable_event_store.rs'),
  await read('core/woml-engine/tests/operational_output_store.rs'),
].join('\n');
for (const marker of [
  'runtime-config.v1.json',
  'runtime-log-record.v1.schema.json',
  'runEventFixtureDirectory',
  'model v3',
  'future_malformed_corrupt_and_unsafe_stores_fail_closed',
  'OperationalOutputStore',
]) {
  if (!compatibility.includes(marker)) {
    throw new Error(`Historical compatibility coverage is missing ${marker}.`);
  }
}

const ci = await read('.github/workflows/ci.yml');
for (const marker of [
  'OpenTelemetry Export (${{ matrix.os }})',
  'ubuntu-24.04',
  'macos-15',
  'windows-2022',
  'bun run test:opentelemetry',
  'opentelemetry-packaged-release.test.ts',
  'bun test ../woml/tests --max-concurrency=1',
  '--test durable_event_store',
  '--test operational_output_store',
]) {
  if (!ci.includes(marker)) throw new Error(`OTE8 CI gate is missing: ${marker}`);
}
const security = await read('.github/workflows/security.yml');
for (const marker of [
  'bun audit --cwd woml-cli',
  'cargo audit --file core/Cargo.lock',
]) {
  if (!security.includes(marker)) {
    throw new Error(`OTE8 locked dependency audit is missing: ${marker}`);
  }
}
const release = await read('.github/workflows/release.yml');
for (const marker of [
  'bun run test:opentelemetry',
  'verify-opentelemetry-release.ts',
]) {
  if (!release.includes(marker)) {
    throw new Error(`OTE8 release workflow gate is missing: ${marker}`);
  }
}

console.log(
  `OpenTelemetry Export release gate passed: ${required.length} public and test artifacts, ` +
    'lazy bundles, frozen budgets, clean packaging, notices, privacy, compatibility, and cross-platform CI verified.'
);
