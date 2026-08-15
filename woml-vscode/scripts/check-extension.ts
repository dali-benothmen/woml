import { readdir } from 'node:fs/promises';
import { join } from 'node:path';

const extensionRoot = join(import.meta.dir, '..');
const requiredFiles = [
  'package.json',
  'language-configuration.json',
  'syntaxes/woml.tmLanguage.json',
  'syntaxes/woml-javascript.injection.tmLanguage.json',
  'syntaxes/woml-references.injection.tmLanguage.json',
  'snippets/woml.code-snippets'
] as const;

for (const relativePath of requiredFiles) {
  const file = Bun.file(join(extensionRoot, relativePath));
  if (!(await file.exists())) {
    throw new Error(`Missing extension file: ${relativePath}`);
  }

  try {
    JSON.parse(await file.text());
  } catch (error) {
    throw new Error(`Invalid JSON in ${relativePath}: ${String(error)}`);
  }
}

const manifest = await Bun.file(join(extensionRoot, 'package.json')).json();
const contributedGrammars = manifest.contributes?.grammars ?? [];
const grammarPaths = new Set(
  contributedGrammars.map((grammar: { path?: string }) => grammar.path)
);

for (const grammarPath of [
  './syntaxes/woml.tmLanguage.json',
  './syntaxes/woml-javascript.injection.tmLanguage.json',
  './syntaxes/woml-references.injection.tmLanguage.json'
]) {
  if (!grammarPaths.has(grammarPath)) {
    throw new Error(`Grammar is not registered in package.json: ${grammarPath}`);
  }
}

const syntaxFiles = (await readdir(join(extensionRoot, 'syntaxes'))).filter(file =>
  file.endsWith('.json')
);

if (syntaxFiles.length !== 3) {
  throw new Error(`Expected 3 grammar files, found ${syntaxFiles.length}.`);
}

console.log('WOML VS Code extension check passed.');
