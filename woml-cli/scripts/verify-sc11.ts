#!/usr/bin/env bun

import { resolve } from 'node:path';

import { compileWoml, parseWoml } from 'woml';

const projectRoot = resolve(import.meta.dir, '../..');
const exampleDirectory = resolve(projectRoot, 'examples/internalEvents');
const models = await Promise.all(
  ['publisher.woml', 'subscriber.woml'].map(async name => {
    const file = resolve(exampleDirectory, name);
    const source = await Bun.file(file).text();
    return compileWoml(parseWoml(source, { file }));
  })
);
const publisher = models.find(model => model.workflowId === 'customer-updater');
const subscriber = models.find(
  model => model.workflowId === 'customer-indexer'
);
if (
  publisher?.schemaVersion !== 8 ||
  subscriber?.schemaVersion !== 7 ||
  subscriber.triggers[0]?.handler !== 'trigger.event' ||
  (subscriber.triggers[0]?.config.kind === 'object' &&
    subscriber.triggers[0].config.fields.secret !== undefined)
) {
  throw new Error(
    'SC11 example must compile one Model v8 publisher and one internal-only Model v7 subscriber.'
  );
}

const schema = await Bun.file(
  resolve(projectRoot, 'docs/schemas/events-service.v1.schema.json')
).json();
if (
  schema['x-status'] !== 'events-service-contract-frozen-sc11' ||
  schema['x-limits']?.lineageDepth !== 32 ||
  schema['x-limits']?.subscribersPerEventName !== 1000
) {
  throw new Error('SC11 Events Service v1 contract is not pinned.');
}

for (const [file, required] of [
  ['docs/woml-events-service.md', '## Internal-only and public events'],
  ['docs/protocols/services-capabilities-v1.md', '## Events Service v1'],
  [
    'WOML Services and Capabilities Implementation Plan.md',
    'Status: completed on 2026-08-10.',
  ],
] as const) {
  const text = await Bun.file(resolve(projectRoot, file)).text();
  if (!text.includes(required)) {
    throw new Error(`SC11 artifact ${file} is missing ${required}.`);
  }
}

process.stdout.write(
  '[SC11] internal event contracts, optional public credentials, example, and lineage policy are publishable\n'
);

await import('./verify-sc10.ts');

process.stdout.write(
  '[SC11] durable internal named-event release gate passed\n'
);
