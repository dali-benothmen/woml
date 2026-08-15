#!/usr/bin/env bun

import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';

import { compileWoml } from '../../woml/src/compiler';
import { parseWoml } from '../../woml/src/parser';

const root = resolve(import.meta.dir, '../..');

async function text(path: string): Promise<string> {
  return readFile(resolve(root, path), 'utf8');
}

const requiredMarkers = new Map<string, readonly string[]>([
  ['docs/woml-v0.1.md', ['<fork>', 'multi-item `<branch>` routes', 'join="none"', 'forkDistributionWorkflow.woml']],
  ['WOML Architecture.md', ['Model v13', 'Event v12', 'fail closed', 'N-API']],
  ['woml-cli/README.md', ['Fork work into independent routes', 'forkDistributionWorkflow.woml']],
]);
for (const [path, markers] of requiredMarkers) {
  const contents = await text(path);
  for (const marker of markers) {
    if (!contents.includes(marker)) {
      throw new Error(`${path} is missing the fork publication marker: ${marker}`);
    }
  }
}

for (const path of [
  'docs/schemas/compiled-workflow-model.v13.schema.json',
  'docs/schemas/run-event.v12.schema.json',
  'docs/schemas/run-inspection.v4.schema.json',
  'docs/schemas/woml-definition-package.v8.schema.json',
]) {
  const schema = JSON.parse(await text(path)) as { 'x-status'?: string };
  if (schema['x-status'] !== 'fork-branch-fj8-published') {
    throw new Error(`${path} is not marked as the published FJ8 contract.`);
  }
}

const editor = JSON.parse(await text('docs/editor/woml-html-data.json')) as {
  tags?: { name?: string; attributes?: { name?: string }[] }[];
};
const editorTags = new Map((editor.tags ?? []).map(tag => [tag.name, tag]));
if (
  !editorTags.has('fork') ||
  !editorTags.has('branch') ||
  !editorTags.get('fork')?.attributes?.some(attribute => attribute.name === 'join')
) {
  throw new Error('The editor declaration does not expose fork, branch, and join.');
}

const sourcePath = resolve(root, 'examples/forkDistributionWorkflow.woml');
const model = compileWoml(
  parseWoml(await readFile(sourcePath, 'utf8'), { file: sourcePath })
) as {
  schemaVersion: number;
  graph: { forks?: { forkId: string; branches: unknown[]; joinedBranchIds: string[] }[] };
};
const distribution = model.graph.forks?.find(fork => fork.forkId === 'distribution');
if (
  model.schemaVersion !== 13 ||
  distribution?.branches.length !== 4 ||
  distribution.joinedBranchIds.length !== 4
) {
  throw new Error('The published example did not lower to the reviewed four-branch Model v13 DAG.');
}

const packageJson = JSON.parse(await text('woml-cli/package.json')) as {
  scripts?: Record<string, string>;
};
if (
  packageJson.scripts?.['test:fork-branch'] === undefined ||
  !packageJson.scripts?.['test:release']?.includes('test:fork-branch')
) {
  throw new Error('The fork release command is absent from the repository release gate.');
}

console.log(
  'Fork and branch publication verification passed: Model v13 example, contracts, editor data, docs, and release wiring are complete.'
);
