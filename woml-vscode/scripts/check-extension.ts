import { readdir } from 'node:fs/promises';
import { join } from 'node:path';

const extensionRoot = join(import.meta.dir, '..');
const requiredFiles = [
  'package.json',
  'language-configuration.json',
  'syntaxes/woml.tmLanguage.json',
  'syntaxes/woml-javascript.injection.tmLanguage.json',
  'syntaxes/woml-references.injection.tmLanguage.json',
  'snippets/woml.code-snippets',
  'icons/woml-file.svg',
  'images/icon.png'
] as const;

for (const relativePath of requiredFiles) {
  const file = Bun.file(join(extensionRoot, relativePath));
  if (!(await file.exists())) {
    throw new Error(`Missing extension file: ${relativePath}`);
  }

  if (relativePath.endsWith('.json') || relativePath.endsWith('.code-snippets')) {
    try {
      JSON.parse(await file.text());
    } catch (error) {
      throw new Error(`Invalid JSON in ${relativePath}: ${String(error)}`);
    }
  }
}

const manifest = await Bun.file(join(extensionRoot, 'package.json')).json();
const contributedGrammars = manifest.contributes?.grammars ?? [];
const language = manifest.contributes?.languages?.find(
  (candidate: { id?: string }) => candidate.id === 'woml'
);
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

if (manifest.icon !== 'images/icon.png') {
  throw new Error('The Marketplace icon must be images/icon.png.');
}

if (
  language?.icon?.light !== './icons/woml-file.svg' ||
  language?.icon?.dark !== './icons/woml-file.svg'
) {
  throw new Error('The WOML language must use the shared file icon for both themes.');
}

const marketplaceIcon = new Uint8Array(
  await Bun.file(join(extensionRoot, 'images/icon.png')).arrayBuffer()
);
const pngSignature = [137, 80, 78, 71, 13, 10, 26, 10];
if (!pngSignature.every((byte, index) => marketplaceIcon[index] === byte)) {
  throw new Error('images/icon.png is not a valid PNG file.');
}

const pngView = new DataView(
  marketplaceIcon.buffer,
  marketplaceIcon.byteOffset,
  marketplaceIcon.byteLength
);
if (pngView.getUint32(16) !== 256 || pngView.getUint32(20) !== 256) {
  throw new Error('images/icon.png must be exactly 256x256 pixels.');
}

const fileIcon = await Bun.file(join(extensionRoot, 'icons/woml-file.svg')).text();
if (!/<svg\b[^>]*\bviewBox=["']0 0 16 16["']/.test(fileIcon)) {
  throw new Error('icons/woml-file.svg must use a 16x16 viewBox.');
}

const syntaxFiles = (await readdir(join(extensionRoot, 'syntaxes'))).filter(file =>
  file.endsWith('.json')
);

if (syntaxFiles.length !== 3) {
  throw new Error(`Expected 3 grammar files, found ${syntaxFiles.length}.`);
}

console.log('WOML VS Code extension check passed.');
