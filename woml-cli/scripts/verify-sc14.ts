#!/usr/bin/env bun

import { resolve } from 'node:path';

import { compileWoml, parseWoml } from 'woml';

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

const plan = await Bun.file(
  resolve(projectRoot, 'WOML Services and Capabilities Implementation Plan.md')
).text();
for (const required of [
  '### SC12 — Freeze the usable queue contract\n\nStatus: postponed',
  '### SC13 — Execute and publish the durable local queue\n\nStatus: postponed',
  '### SC14 — Complete Services and Capabilities\n\nStatus: completed',
]) {
  if (!plan.includes(required)) {
    throw new Error(`SC14 implementation plan is missing: ${required}`);
  }
}

const language = await Bun.file(
  resolve(projectRoot, 'docs/woml-v0.1.md')
).text();
const architecture = await Bun.file(
  resolve(projectRoot, 'docs/architecture.md')
).text();
const migration = await Bun.file(
  resolve(projectRoot, 'docs/woml-sdk-migration.md')
).text();
if (
  !language.includes('SC0–SC14 completed and hardened') ||
  !architecture.includes('Queue remains intentionally postponed') ||
  !migration.includes('Queue is explicitly postponed')
) {
  throw new Error('SC14 architecture, language, or migration status is stale.');
}

process.stdout.write(
  '[SC14] five built-ins, native Fetch, composition example, docs, and queue deferral are publishable\n'
);

await import('./verify-sc11.ts');

process.stdout.write(
  '[SC14] complete Services and Capabilities release gate passed\n'
);
