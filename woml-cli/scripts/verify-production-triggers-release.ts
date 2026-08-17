#!/usr/bin/env bun

import { resolve } from 'node:path';

import { compileWoml, parseWoml } from 'woml';

const cliRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(cliRoot, '..');
const examplePaths = [
  'webhookWorkflow.woml',
  'slackTriggerWorkflow.woml',
  'scheduleWorkflow.woml',
  'intervalWorkflow.woml',
  'eventWorkflow.woml',
].map(name => resolve(projectRoot, 'examples', name));
const workflows = await Promise.all(
  examplePaths.map(async path =>
    compileWoml(parseWoml(await Bun.file(path).text(), { file: path }))
  )
);
const handlers = new Set(
  workflows.flatMap(workflow =>
    workflow.triggers.map(trigger => trigger.handler)
  )
);
for (const handler of [
  'trigger.webhook',
  'trigger.slack',
  'trigger.schedule',
  'trigger.interval',
  'trigger.event',
]) {
  if (!handlers.has(handler)) {
    throw new Error(
      `T13 verification failed: release examples do not cover ${handler}.`
    );
  }
}

const operationsPath = resolve(
  projectRoot,
  'docs/woml-production-triggers.md'
);
const protocolPath = resolve(
  projectRoot,
  'docs/protocols/production-triggers-v1.md'
);
const [operations, protocol] = await Promise.all([
  Bun.file(operationsPath).text(),
  Bun.file(protocolPath).text(),
]);
for (const section of [
  '## Secrets',
  '## State, restart, and definition changes',
  '## Shutdown and single-node boundary',
  '## Troubleshooting',
  '## Deployment checklist',
]) {
  if (!operations.includes(section)) {
    throw new Error(
      `T13 verification failed: operations guide is missing ${section}.`
    );
  }
}
if (!protocol.includes('completed in T13')) {
  throw new Error(
    'T13 verification failed: protocol status documentation is stale.'
  );
}

process.stdout.write(
  '[T13] all production-trigger examples, operations, and protocol surfaces are complete\n'
);

// The event gate supplies the shared frontend, Rust, isolated CLI, schema,
// packaging, recovery, contention, and secret-safety checks. This gate adds the
// all-trigger coexistence journey.
await import('./verify-event-release.ts');

process.stdout.write('[T13] complete Production Triggers release gate passed\n');
