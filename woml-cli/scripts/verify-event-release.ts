#!/usr/bin/env bun

import { resolve } from 'node:path';

import { compileWoml, parseWoml } from '@woml/compiler';

const cliRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(cliRoot, '..');
const examples = [
  resolve(projectRoot, 'examples/events/sendConfirmation.woml'),
  resolve(projectRoot, 'examples/events/updateInventory.woml'),
];
const workflows = await Promise.all(
  examples.map(async path =>
    compileWoml(parseWoml(await Bun.file(path).text(), { file: path }))
  )
);
const payload = await Bun.file(
  resolve(projectRoot, 'examples/events/order-created.json')
).json();

if (
  workflows[0]?.workflowId !== 'send-confirmation' ||
  workflows[1]?.workflowId !== 'update-inventory' ||
  workflows.some(
    workflow =>
      workflow.triggers.length !== 1 ||
      workflow.triggers[0]?.handler !== 'trigger.event' ||
      workflow.triggers[0]?.config.kind !== 'object' ||
      workflow.triggers[0]?.config.fields.name?.kind !== 'literal' ||
      workflow.triggers[0]?.config.fields.name.value !== 'order.created'
  ) ||
  payload.orderId !== 'order-42'
) {
  throw new Error(
    'T12 verification failed: the two-workflow event product journey drifted.'
  );
}

process.stdout.write(
  '[T12] two exact-name subscribers and their sample publication are pinned\n'
);

// The event-authoring and notification gates supply the shared native, Rust,
// and isolated CLI checks. This gate adds durable fan-out.
await import('./verify-event-authoring.ts');

process.stdout.write('[T12] durable named-event release gate passed\n');
