#!/usr/bin/env bun

import { resolve } from 'node:path';

import { compileWoml, parseWoml } from 'woml';

const cliRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(cliRoot, '..');
const examplePath = resolve(projectRoot, 'examples', 'webhookWorkflow.woml');
const source = await Bun.file(examplePath).text();
const workflow = compileWoml(parseWoml(source, { file: examplePath }));
const webhooks = workflow.triggers.filter(
  trigger => trigger.handler === 'trigger.webhook'
);

if (workflow.schemaVersion !== 7 || webhooks.length !== 1) {
  throw new Error(
    'T5 verification failed: examples/webhookWorkflow.woml must compile to Model v7 with exactly one webhook trigger.'
  );
}
if (!source.includes('<schema>') || !source.includes('auth="none"')) {
  throw new Error(
    'T5 verification failed: the release webhook example must demonstrate an inline schema and explicit authentication policy.'
  );
}

process.stdout.write(
  '[T5] production webhook example compiles through the frozen Model v7 contract\n'
);

// RI7 owns the existing complete release gate. Its N6 verifier rebuilds the
// package, runs every frontend/Rust/CLI suite in isolation, and scans public and
// durable artifacts for active WOML secrets. T5 extends that gate through the
// webhook hardening/runtime tests and the long-lived example smoke journey.
await import('./verify-ri7.ts');

process.stdout.write('[T5] production webhook release gate passed\n');
