#!/usr/bin/env bun

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

import {
  generateWomlEditorDeclarations,
  inspectWomlModuleUsage,
  parseWoml,
} from '@woml/compiler';

const repositoryRoot = resolve(import.meta.dir, '../..');
const fixture = resolve(
  repositoryRoot,
  'woml/tests/fixtures/modules/customer-import.woml'
);
const inspection = inspectWomlModuleUsage(
  parseWoml(readFileSync(fixture, 'utf8'), { file: fixture })
);
if (
  inspection.referencedModules.join(',') !== 'spreadsheet' ||
  inspection.unusedModules.length !== 0
) {
  throw new Error('Essential MS6 module-usage inspection drifted.');
}

const declarations = generateWomlEditorDeclarations([
  { name: 'spreadsheet', exports: ['read', 'removeEmptyRows'] },
]);
for (const required of [
  'declare const services',
  'readonly http',
  'readonly db',
  'readonly storage',
  'readonly cache',
  'readonly events',
  'readonly workflows',
  'readonly call',
  'readonly "spreadsheet"',
]) {
  if (!declarations.includes(required)) {
    throw new Error(`Essential MS6 declarations are missing ${required}.`);
  }
}
for (const forbidden of [
  'declare const context',
  'declare const attempt',
  'declare const secrets',
]) {
  if (declarations.includes(forbidden)) {
    throw new Error(
      `Essential MS6 declarations expose forbidden ${forbidden}.`
    );
  }
}

const guide = readFileSync(
  resolve(repositoryRoot, 'docs/woml-modules.md'),
  'utf8'
);
if (
  !guide.includes('## Editor autocomplete is automatic') ||
  !guide.includes('`woml run` refreshes `woml-env.d.ts`') ||
  !guide.includes('## Unit-test a module')
) {
  throw new Error('The module authoring guide is incomplete.');
}

process.stdout.write(
  '[modules] editor types, alias diagnostics, mocked tests, and authoring guidance are publishable\n'
);
