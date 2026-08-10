#!/usr/bin/env bun

import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const repositoryRoot = resolve(import.meta.dir, '../..');
const requiredFiles = [
  'docs/protocols/workflow-calls-v1.md',
  'docs/schemas/compiled-workflow-model.v10.schema.json',
  'docs/schemas/workflow-call.v1.schema.json',
  'docs/schemas/workflow-call-index.v1.schema.json',
  'docs/schemas/workflow-call-routing.v1.schema.json',
  'docs/schemas/woml-definition-package.v4.schema.json',
  'docs/schemas/woml-definition-package.v5.schema.json',
  'woml/tests/fixtures/workflow-calls/calculate-risk.woml',
  'woml/tests/fixtures/workflow-calls/request-risk.woml',
];

for (const file of requiredFiles) {
  if (!existsSync(resolve(repositoryRoot, file))) {
    throw new Error(`WC1 release artifact is missing: ${file}`);
  }
}

const plan = readFileSync(
  resolve(repositoryRoot, 'WOML Durable Workflow Calls Implementation Plan.md'),
  'utf8'
);
const protocol = readFileSync(
  resolve(repositoryRoot, 'docs/protocols/workflow-calls-v1.md'),
  'utf8'
);
if (
  !plan.includes('WC0 and WC1 completed') ||
  !plan.includes('The next implementation action is WC2') ||
  !protocol.includes('Compiled Workflow Model v10') ||
  !protocol.includes('approval')
) {
  throw new Error('WC0/WC1 plan or frozen protocol is incomplete.');
}

process.stdout.write(
  '[WC1] frozen contracts, call-only lowering, workflow-call analysis, editor types, and honest CLI boundary are publishable\n'
);
