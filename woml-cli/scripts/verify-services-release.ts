#!/usr/bin/env bun

import { resolve } from 'node:path';

import { compileWoml, parseWoml } from '@woml/compiler';

const projectRoot = resolve(import.meta.dir, '../..');
const exampleDirectory = resolve(projectRoot, 'examples/servicesComposition');
const models = await Promise.all(
  ['publisher.woml', 'subscriber.woml'].map(async name => {
    const file = resolve(exampleDirectory, name);
    const source = await Bun.file(file).text();
    return compileWoml(parseWoml(source, { file }));
  })
);
const publisher = models.find(
  model => model.workflowId === 'services-order-publisher'
);
const subscriber = models.find(
  model => model.workflowId === 'services-order-subscriber'
);
if (
  publisher?.schemaVersion !== 8 ||
  publisher.graph.nodes.length !== 2 ||
  publisher.graph.nodes.some(
    node => node.scriptRuntime?.bindings.includes('services') !== true
  ) ||
  subscriber?.schemaVersion !== 7 ||
  subscriber.triggers[0]?.handler !== 'trigger.event' ||
  (subscriber.triggers[0]?.config.kind === 'object' &&
    subscriber.triggers[0].config.fields.secret !== undefined)
) {
  throw new Error(
    'SC14 composition example must compile one two-step Model v8 publisher and one internal-only Model v7 subscriber.'
  );
}

const guide = await Bun.file(
  resolve(projectRoot, 'docs/woml-services.md')
).text();
for (const required of [
  '## Choose the smallest useful service',
  '## Reliability rules authors should know',
  '## Run the examples',
  'Queue is deliberately unavailable.',
]) {
  if (!guide.includes(required)) {
    throw new Error(`SC14 services guide is missing: ${required}`);
  }
}

const language = await Bun.file(
  resolve(projectRoot, 'docs/language-reference.md')
).text();
const architecture = await Bun.file(
  resolve(projectRoot, 'docs/architecture.md')
).text();
if (
  !language.includes('`services.http.request()` returns') ||
  !architecture.includes('Managed capabilities use a full-duplex Rust/Bun')
) {
  throw new Error('SC14 architecture or language status is stale.');
}

process.stdout.write(
  '[services] five built-ins, native Fetch, composition example, docs, and queue deferral are publishable\n'
);

await import('./verify-events-release.ts');

process.stdout.write(
  '[services] complete Services and Capabilities release gate passed\n'
);
