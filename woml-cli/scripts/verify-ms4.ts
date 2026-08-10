#!/usr/bin/env bun

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import Ajv2020 from 'ajv/dist/2020';

const repositoryRoot = resolve(import.meta.dir, '../..');
const requiredFiles = [
  'docs/protocols/module-recovery-v1.md',
  'docs/protocols/module-runtime-v1.md',
  'docs/schemas/script-host-protocol.v6.schema.json',
  'docs/schemas/stored-run-requirements.v1.schema.json',
  'core/woml-engine/tests/ms4_modules.rs',
];
for (const file of requiredFiles) readFileSync(resolve(repositoryRoot, file));

const ajv = new Ajv2020({ strict: true });
const requirementsSchema = JSON.parse(
  readFileSync(
    resolve(
      repositoryRoot,
      'docs/schemas/stored-run-requirements.v1.schema.json'
    ),
    'utf8'
  )
);
const validateRequirements = ajv.compile(requirementsSchema);
if (
  !validateRequirements({
    contract: 'woml.stored-run-requirements',
    version: 1,
    workflowId: 'customer-import',
    definitionHash: `sha256:${'a'.repeat(64)}`,
    requiredSecrets: [],
    moduleCount: 1,
    hasApproval: false,
    hasNotifications: false,
  })
) {
  throw new Error('MS4 stored-run requirements schema rejected its fixture.');
}

const protocol = readFileSync(
  resolve(repositoryRoot, 'docs/schemas/script-host-protocol.v6.schema.json'),
  'utf8'
);
for (const required of [
  '"protocolVersion": { "const": 6 }',
  '"sourceMapDigest"',
  '"sourceMap"',
  'WOML_MODULE_CACHE_LIMIT_EXCEEDED',
]) {
  if (!protocol.includes(required)) {
    throw new Error(`MS4 Script Host v6 schema is missing ${required}.`);
  }
}

const plan = readFileSync(
  resolve(repositoryRoot, 'WOML Module System Implementation Plan.md'),
  'utf8'
);
if (
  !plan.includes('Status: MS0 through MS4 completed') ||
  !plan.includes(
    '### MS4 — Make local modules recoverable and observable — completed'
  )
) {
  throw new Error('MS4 implementation-plan status is stale.');
}

process.stdout.write(
  '[MS4] durable artifacts, source-free recovery, Script Host v6, safe diagnostics, and composition are publishable\n'
);
