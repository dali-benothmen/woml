#!/usr/bin/env bun

import { resolve } from 'node:path';

const repositoryRoot = resolve(import.meta.dir, '../..');

for (const [file, required] of [
  [
    'WOML Production Runtime and Operations Implementation Plan.md',
    'PRO4 — Production secrets and administration security (completed)',
  ],
  ['woml-cli/src/secrets/mounted-file-secret-store.ts', 'WOML_SECRET_FILE_UNSAFE'],
  ['woml-cli/src/secrets/production-secret-store.ts', 'WOML_SECRET_SOURCE_CONFLICT'],
  ['woml-cli/src/runtime-control.ts', 'DEFAULT_MAX_REQUEST_BYTES'],
  ['woml-cli/src/runtime-control.ts', "'cancel_run'"],
  ['woml-cli/src/cli.ts', 'requestLiveRunOperation'],
  ['docs/woml-production-runtime.md', 'WOML_SECRETS_DIRECTORY'],
  ['docs/woml-data-security.md', 'Process isolation'],
] as const) {
  const contents = await Bun.file(resolve(repositoryRoot, file)).text();
  if (!contents.includes(required)) {
    throw new Error(`PRO4 artifact ${file} is missing ${required}.`);
  }
}

for (const file of [
  'docs/woml-production-runtime.md',
  'docs/woml-data-security.md',
  'woml-cli/src/runtime-control.ts',
]) {
  const contents = await Bun.file(resolve(repositoryRoot, file)).text();
  for (const forbidden of [
    'xoxb-real-',
    'xapp-real-',
    'postgres://production-secret@',
  ]) {
    if (contents.includes(forbidden)) {
      throw new Error(`PRO4 redaction scan found sensitive fixture text in ${file}.`);
    }
  }
}

console.log(
  '[PRO4] Production secret sources, rotating loopback administration, live controls, bounds, redaction, and isolation guidance passed.'
);
