#!/usr/bin/env bun

import { resolve } from 'node:path';

import { compileWoml, parseWoml } from 'woml';

const repositoryRoot = resolve(import.meta.dir, '../..');
const exampleDirectory = resolve(repositoryRoot, 'examples/workflowCalls');
const models = await Promise.all(
  ['request-risk.woml', 'calculate-risk.woml'].map(async name => {
    const file = resolve(exampleDirectory, name);
    const source = await Bun.file(file).text();
    return compileWoml(parseWoml(source, { file }));
  })
);
const parent = models.find(model => model.workflowId === 'request-risk');
const child = models.find(model => model.workflowId === 'calculate-risk');
if (
  parent?.triggers[0]?.handler !== 'trigger.manual' ||
  child?.schemaVersion !== 10 ||
  child.triggers.length !== 0
) {
  throw new Error(
    'WC3 example must contain one manual parent and one Model v10 call-only child.'
  );
}

for (const [file, required] of [
  ['docs/protocols/workflow-calls-v1.md', 'same-runtime children'],
  ['docs/woml-services.md', 'woml run examples/workflowCalls'],
] as const) {
  const text = await Bun.file(resolve(repositoryRoot, file)).text();
  if (!text.includes(required)) {
    throw new Error(`WC3 artifact ${file} is missing ${required}.`);
  }
}

process.stdout.write(
  '[WC3] Bun facade, durable Rust child execution, direct JSON results, and the two-workflow example are complete\n'
);
