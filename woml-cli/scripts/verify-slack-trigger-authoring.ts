#!/usr/bin/env bun

import { resolve } from 'node:path';

import { compileWoml, parseWoml } from 'woml';

const cliRoot = resolve(import.meta.dir, '..');
const projectRoot = resolve(cliRoot, '..');
const sourcePath = resolve(
  projectRoot,
  'woml',
  'tests',
  'fixtures',
  'triggers-slack.woml'
);
const modelPath = resolve(
  projectRoot,
  'woml',
  'tests',
  'fixtures',
  'triggers-slack.compiled.v7.json'
);
const manifestPath = resolve(cliRoot, 'slack', 'manifest.json');

const source = await Bun.file(sourcePath).text();
const expected = await Bun.file(modelPath).json();
const compiled = compileWoml(parseWoml(source, { file: sourcePath }));
if (JSON.stringify(compiled) !== JSON.stringify(expected)) {
  throw new Error(
    'T6 verification failed: the reviewed Slack source does not deep-equal its Model v7 fixture.'
  );
}

const manifest = await Bun.file(manifestPath).json();
const scopes = manifest.oauth_config?.scopes?.bot;
const subscriptions = manifest.settings?.event_subscriptions?.bot_events;
for (const scope of ['app_mentions:read', 'im:history']) {
  if (!Array.isArray(scopes) || !scopes.includes(scope)) {
    throw new Error(`T6 verification failed: Slack manifest lacks ${scope}.`);
  }
}
for (const event of ['app_mention', 'message.im']) {
  if (!Array.isArray(subscriptions) || !subscriptions.includes(event)) {
    throw new Error(
      `T6 verification failed: Slack manifest lacks ${event} subscription.`
    );
  }
}

process.stdout.write(
  '[T6] reviewed Slack syntax, Model v7, scopes, and subscriptions are pinned\n'
);

// The webhook gate supplies the shared frontend, Rust, and CLI checks. This
// gate adds Slack transport and approval compatibility.
await import('./verify-webhook-release.ts');

process.stdout.write('[T6] Slack compilation and shared transport gate passed\n');
