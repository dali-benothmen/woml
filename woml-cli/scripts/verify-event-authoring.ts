#!/usr/bin/env bun

import { resolve } from 'node:path';

import { compileWoml, parseWoml } from 'woml';

const cliRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(cliRoot, '..');
const examplePath = resolve(projectRoot, 'examples/eventWorkflow.woml');
const reviewedSourcePath = resolve(
  projectRoot,
  'woml/tests/fixtures/triggers-event.woml'
);
const reviewedModelPath = resolve(
  projectRoot,
  'woml/tests/fixtures/triggers-event.compiled.v7.json'
);
const publicationSchemaPath = resolve(
  projectRoot,
  'docs/schemas/event-publication.v1.schema.json'
);
const httpSchemaPath = resolve(
  projectRoot,
  'docs/schemas/event-publisher-http.v1.schema.json'
);
const [example, reviewedSource, reviewedModel, publicationSchema, httpSchema] =
  await Promise.all([
    Bun.file(examplePath).text(),
    Bun.file(reviewedSourcePath).text(),
    Bun.file(reviewedModelPath).json(),
    Bun.file(publicationSchemaPath).json(),
    Bun.file(httpSchemaPath).json(),
  ]);

if (example !== reviewedSource) {
  throw new Error(
    'T11 verification failed: examples/eventWorkflow.woml must exactly match the reviewed event fixture.'
  );
}
const compiled = compileWoml(parseWoml(example, { file: examplePath }));
if (JSON.stringify(compiled) !== JSON.stringify(reviewedModel)) {
  throw new Error(
    'T11 verification failed: the event product example does not deep-equal Model v7.'
  );
}
if (
  publicationSchema.$id !==
    'https://cronflow.dev/schemas/event-publication/v1' ||
  httpSchema.$id !==
    'https://cronflow.dev/schemas/event-publisher-http/v1' ||
  httpSchema['x-http-request']?.path !== '/_woml/events/{eventName}'
) {
  throw new Error(
    'T11 verification failed: Event Publication v1 or its HTTP mapping drifted.'
  );
}

process.stdout.write(
  '[T11] event syntax, Model v7, publication, fan-out, and HTTP contracts are pinned\n'
);

// The interval and notification gates supply the shared cross-layer,
// packaging, and secret checks. This gate adds the event publication contract
// and authentication boundary.
await import('./verify-interval-release.ts');

process.stdout.write('[T11] named-event compiler and contract release gate passed\n');
