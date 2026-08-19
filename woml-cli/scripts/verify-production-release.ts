#!/usr/bin/env bun

import { readdir, readFile, stat } from 'node:fs/promises';
import { extname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

import Ajv2020 from 'ajv/dist/2020.js';
import addFormats from 'ajv-formats';

const root = new URL('../../', import.meta.url);
const rootPath = fileURLToPath(root);

async function filesBelow(directory: string): Promise<string[]> {
  const files: string[] = [];
  const visit = async (path: string): Promise<void> => {
    for (const entry of await readdir(path, { withFileTypes: true })) {
      const child = join(path, entry.name);
      if (entry.isDirectory()) await visit(child);
      else files.push(child);
    }
  };
  await visit(directory);
  return files.sort();
}

const required = new Map([
  ['docs/woml-production-deployment.md', ['systemd on a VPS', 'Single-pod Kubernetes', 'Security checklist']],
  ['examples/production/deployment/Dockerfile', ['USER woml', 'woml.runtime.json']],
  ['examples/production/deployment/woml.service', ['NoNewPrivileges=true', 'ExecStartPre=']],
  ['examples/production/deployment/nginx.conf', ['proxy_pass http://127.0.0.1:3000']],
  ['examples/production/deployment/kubernetes.yaml', ['replicas: 1', 'ReadWriteOnce', 'readOnlyRootFilesystem: true']],
  ['examples/production/deployment/prometheus-alerts.yaml', ['WomlRuntimeNotReady', 'WomlRetentionFailure']],
  ['examples/production/complete/README.md', ['complete Production Runtime v1 surface']],
  ['woml-cli/tests/production-runtime-release.test.ts', ['a clean consumer installs, activates, serves, observes, backs up, stops, restores, and prunes']],
]);
for (const [path, markers] of required) {
  const contents = await readFile(join(rootPath, path), 'utf8');
  for (const marker of markers) {
    if (!contents.includes(marker)) throw new Error(`${path} is missing PRO9 marker: ${marker}`);
  }
}

const schemaDirectory = join(rootPath, 'docs/schemas');
const schemas = await Promise.all(
  (await filesBelow(schemaDirectory))
    .filter(path => extname(path) === '.json')
    .map(async path => ({ path, value: JSON.parse(await readFile(path, 'utf8')) }))
);
const ajv = new Ajv2020({ strict: false, allErrors: true });
addFormats(ajv);
for (const { path, value } of schemas) {
  if (!ajv.validateSchema(value)) {
    throw new Error(`Invalid published schema ${relative(rootPath, path)}: ${JSON.stringify(ajv.errors)}`);
  }
  ajv.addSchema(value);
}

const jsonFixtureRoots = [
  join(rootPath, 'woml/tests/fixtures'),
  join(rootPath, 'woml-cli/tests/fixtures'),
];
let parsedFixtures = 0;
for (const fixtureRoot of jsonFixtureRoots) {
  for (const path of await filesBelow(fixtureRoot)) {
    if (extname(path) !== '.json') continue;
    JSON.parse(await readFile(path, 'utf8'));
    parsedFixtures += 1;
  }
}

const compiledFixtures = (await filesBelow(join(rootPath, 'woml/tests/fixtures')))
  .filter(path => /\.compiled\.v\d+\.json$/.test(path));
for (const path of compiledFixtures) {
  const fixture = JSON.parse(await readFile(path, 'utf8')) as { schemaVersion: number };
  const validate = ajv.getSchema(
    `https://cronflow.dev/schemas/compiled-workflow-model/v${fixture.schemaVersion}`
  );
  if (validate === undefined || !validate(fixture)) {
    throw new Error(`Historical model failed ${relative(rootPath, path)}: ${JSON.stringify(validate?.errors)}`);
  }
}

const eventFixtureRoots = [
  join(rootPath, 'woml/tests/fixtures/run-events'),
  join(rootPath, 'woml/tests/fixtures/workflow-call-events'),
  join(rootPath, 'woml/tests/fixtures/lifecycle'),
  join(rootPath, 'woml/tests/fixtures/runtime-policies'),
];
let validatedEvents = 0;
for (const fixtureRoot of eventFixtureRoots) {
  for (const path of await filesBelow(fixtureRoot)) {
    if (!/(?:event|events)\.v\d+\.json$/.test(path)) continue;
    const decoded = JSON.parse(await readFile(path, 'utf8'));
    const events = Array.isArray(decoded) ? decoded : [decoded];
    for (const event of events) {
      if (event === null || typeof event !== 'object' || typeof event.eventSchemaVersion !== 'number') continue;
      const validate = ajv.getSchema(
        `https://cronflow.dev/schemas/run-event/v${event.eventSchemaVersion}`
      );
      if (validate === undefined || !validate(event)) {
        throw new Error(`Historical event failed ${relative(rootPath, path)}: ${JSON.stringify(validate?.errors)}`);
      }
      validatedEvents += 1;
    }
  }
}

const packageJson = JSON.parse(
  await readFile(join(rootPath, 'woml-cli/package.json'), 'utf8')
);
if (
  packageJson.name !== 'woml-cli' ||
  packageJson.version !== '1.0.6' ||
  packageJson.private !== false ||
  packageJson.bin?.woml !== './dist/cli.js' ||
  !packageJson.files?.includes('dist') ||
  packageJson.license !== 'Apache-2.0'
) {
  throw new Error('The woml-cli package metadata is not publishable as Production Runtime v1.');
}
const distBytes = (
  await Promise.all(
    (await filesBelow(join(rootPath, 'woml-cli/dist'))).map(async path =>
      (await stat(path)).size
    )
  )
).reduce((sum, size) => sum + size, 0);
if (distBytes > 25 * 1024 * 1024) {
  throw new Error(`Installed runtime exceeds the 25 MiB PRO9 package budget: ${distBytes} bytes.`);
}

const forbidden = [
  /xox[baprs]-live-[A-Za-z0-9-]+/,
  /sk-live-[A-Za-z0-9-]+/,
  /-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----/,
];
for (const directory of ['woml-cli/dist', 'docs', 'examples/production']) {
  for (const path of await filesBelow(join(rootPath, directory))) {
    if (path.endsWith('.node')) continue;
    const contents = await readFile(path, 'utf8').catch(() => '');
    for (const pattern of forbidden) {
      if (pattern.test(contents)) {
        throw new Error(`Active secret-like value found in ${relative(rootPath, path)}.`);
      }
    }
  }
}

console.log(
  `PRO9 verification passed: ${schemas.length} schemas, ${parsedFixtures} JSON fixtures, ${compiledFixtures.length} historical models, ${validatedEvents} historical events, ${(distBytes / 1024 / 1024).toFixed(2)} MiB runtime.`
);
