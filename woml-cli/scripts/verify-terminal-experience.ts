#!/usr/bin/env bun

import { readdir, readFile, stat } from 'node:fs/promises';
import { extname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

import { compileWoml, parseWoml } from '@woml/compiler';

const rootPath = fileURLToPath(new URL('../../', import.meta.url));

async function filesBelow(directory: string): Promise<string[]> {
  const output: string[] = [];
  const visit = async (current: string): Promise<void> => {
    for (const entry of await readdir(current, { withFileTypes: true })) {
      const path = join(current, entry.name);
      if (entry.isDirectory()) await visit(path);
      else output.push(path);
    }
  };
  await visit(directory);
  return output.sort();
}

const required = new Map<string, readonly string[]>([
  ['docs/woml-terminal-experience.md', [
    'woml run workflow.woml',
    'woml test workflow.woml',
    'woml <run-id|workflow-id> --logs',
    'status symbols',
    'NO_COLOR',
    'WOML_LOG_STATE_UNAVAILABLE',
  ]],
  ['woml-cli/README.md', [
    'press Enter for each independent durable',
    'WOML Terminal Experience',
  ]],
  ['examples/terminalExperience/README.md', [
    'Sequential and repeated manual runs',
    'Intentional failure',
    'Background logs',
  ]],
  ['examples/terminalExperience/sequential.woml', ['id="terminal-sequential"']],
  ['examples/terminalExperience/failure.woml', ['id="terminal-failure"']],
  ['woml-cli/tests/terminal-experience-packaged-release.test.ts', [
    'clean installed terminal experience',
  ]],
  ['woml-cli/scripts/benchmark-terminal-experience.ts', [
    'rendererAverageMs',
    'projectionAverageMs',
    'retainedAttachAverageMs',
    'memoryPerActiveRunBytes',
  ]],
]);
for (const [path, markers] of required) {
  const contents = await readFile(join(rootPath, path), 'utf8');
  for (const marker of markers) {
    if (!contents.includes(marker)) {
      throw new Error(`${path} is missing terminal release marker: ${marker}`);
    }
  }
}

const staleClaims = [
  '`woml run` executes a selected manual trigger once at startup',
  'Manual triggers remain available for startup',
];
for (const path of ['woml-cli/README.md', 'docs/woml-production-triggers.md']) {
  const contents = await readFile(join(rootPath, path), 'utf8');
  for (const claim of staleClaims) {
    if (contents.includes(claim)) {
      throw new Error(`${path} retains stale manual behavior: ${claim}`);
    }
  }
}

for (const relativePath of [
  'examples/terminalExperience/sequential.woml',
  'examples/terminalExperience/failure.woml',
]) {
  const path = join(rootPath, relativePath);
  const compiled = compileWoml(parseWoml(await readFile(path, 'utf8'), { file: path }));
  if (compiled.graph.nodes.length < 2 || compiled.triggers[0]?.handler !== 'trigger.manual') {
    throw new Error(`${relativePath} does not compile as a real multi-step manual workflow.`);
  }
}

const budgets = JSON.parse(await readFile(join(
  rootPath,
  'examples/terminalExperience/performance-budgets.v1.json'
), 'utf8')) as Record<string, unknown>;
if (
  budgets.profile !== 'woml.terminal-performance-budgets/v1' ||
  Object.entries(budgets).some(([key, value]) =>
    key !== 'profile' && (typeof value !== 'number' || value <= 0)
  )
) {
  throw new Error('Terminal performance budgets are missing, invalid, or unversioned.');
}

const packageJson = JSON.parse(await readFile(
  join(rootPath, 'woml-cli/package.json'),
  'utf8'
)) as { scripts?: Record<string, string> };
for (const command of [
  'test:terminal-foundation',
  'test:run-presentation',
  'test:manual-triggers',
  'test:log-following',
  'test:terminal-hardening',
  'test:terminal-package',
  'test:terminal-release',
  'benchmark:terminal-experience',
  'verify:terminal-experience',
]) {
  if (packageJson.scripts?.[command] === undefined) {
    throw new Error(`woml-cli/package.json is missing release command ${command}.`);
  }
}

const dist = join(rootPath, 'woml-cli/dist');
const artifacts = await filesBelow(dist);
for (const name of [
  'cli.js',
  'script-host.js',
  'script-host-worker.js',
  `woml-core.${process.platform}-${process.arch}.node`,
]) {
  const path = join(dist, name);
  if (!artifacts.includes(path) || (await stat(path)).size === 0) {
    throw new Error(`Packaged terminal runtime is missing ${name}.`);
  }
}

const forbidden = [
  /xox[baprs]-live-[A-Za-z0-9-]+/u,
  /xapp-live-[A-Za-z0-9-]+/u,
  /sk-live-[A-Za-z0-9-]+/u,
  /-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----/u,
];
let scanned = 0;
for (const directory of [
  'docs',
  'examples/terminalExperience',
  'woml-cli/tests/fixtures/terminal-presentation',
]) {
  for (const path of await filesBelow(join(rootPath, directory))) {
    if (!['.md', '.txt', '.json', '.woml'].includes(extname(path))) continue;
    const contents = await readFile(path, 'utf8');
    for (const pattern of forbidden) {
      if (pattern.test(contents)) {
        throw new Error(`Active secret-shaped value found in ${relative(rootPath, path)}.`);
      }
    }
    scanned += 1;
  }
}

console.log(
  `Terminal release verification passed: ${required.size} release artifacts, ` +
  `2 compiled examples, ${Object.keys(budgets).length - 1} performance budgets, ` +
  `${scanned} documentation/fixture files scanned.`
);
