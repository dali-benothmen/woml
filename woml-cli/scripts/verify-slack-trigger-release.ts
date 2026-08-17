#!/usr/bin/env bun

import { resolve } from 'node:path';

import { compileWoml, parseWoml } from 'woml';

const cliRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(cliRoot, '..');
const productExample = resolve(
  projectRoot,
  'examples',
  'slackTriggerWorkflow.woml'
);
const exampleSource = await Bun.file(productExample).text();
const compiled = compileWoml(
  parseWoml(exampleSource, { file: productExample })
);
const slackTriggers = compiled.triggers.filter(
  trigger => trigger.handler === 'trigger.slack'
);
if (
  compiled.schemaVersion !== 7 ||
  slackTriggers.length !== 1 ||
  !exampleSource.includes('channels="woml-testing"') ||
  exampleSource.includes('xoxb-') ||
  exampleSource.includes('xapp-')
) {
  throw new Error(
    'T7 verification failed: the packaged Slack example must use Model v7, one Slack trigger, the test channel, and symbolic credentials.'
  );
}

process.stdout.write(
  '[T7] packaged Slack trigger example compiles through Model v7 with symbolic credentials\n'
);

// The Slack authoring gate rebuilds the package, scans it for secrets, and
// runs the provider's isolated CLI, fake-Slack, and native Rust journeys.
await import('./verify-slack-trigger-authoring.ts');

process.stdout.write('[T7] Slack trigger release gate passed\n');
