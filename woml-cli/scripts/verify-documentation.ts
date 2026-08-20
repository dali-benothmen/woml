#!/usr/bin/env bun

import {
  cpSync,
  existsSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  statSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, extname, relative, resolve } from 'node:path';

const repositoryRoot = resolve(import.meta.dir, '../..');
const failures: string[] = [];

const ignoredDirectories = new Set([
  '.git',
  '.woml',
  'dist',
  'node_modules',
  'target',
]);

function filesUnder(directory: string, extension: string): string[] {
  const result: string[] = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (entry.isDirectory() && ignoredDirectories.has(entry.name)) continue;
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) result.push(...filesUnder(path, extension));
    else if (entry.isFile() && extname(entry.name) === extension) result.push(path);
  }
  return result.sort();
}

function markdownTargets(source: string): string[] {
  const targets: string[] = [];
  const prose = source
    .replace(/```[\s\S]*?```/g, '')
    .replace(/`[^`\n]*`/g, '');
  for (const match of prose.matchAll(/!?\[[^\]]*\]\(([^)]+)\)/g)) {
    const raw = match[1].trim();
    const target = raw.startsWith('<') && raw.endsWith('>')
      ? raw.slice(1, -1)
      : raw.split(/\s+["']/u, 1)[0];
    if (target.length > 0) targets.push(target);
  }
  for (const match of prose.matchAll(/(?:href|src)=["']([^"']+)["']/g)) {
    targets.push(match[1]);
  }
  return targets;
}

for (const markdown of filesUnder(repositoryRoot, '.md')) {
  const relativePath = relative(repositoryRoot, markdown);
  if (relativePath === 'woml-cli/README.md') {
    continue;
  }
  const source = readFileSync(markdown, 'utf8');
  for (const target of markdownTargets(source)) {
    if (
      target.startsWith('#') ||
      /^[a-z][a-z0-9+.-]*:/iu.test(target) ||
      target.startsWith('//')
    ) {
      continue;
    }
    const pathPart = target.split('#', 1)[0];
    if (pathPart.length === 0) continue;
    let decoded: string;
    try {
      decoded = decodeURIComponent(pathPart);
    } catch {
      failures.push(`${markdown}: malformed link target ${target}`);
      continue;
    }
    const resolved = resolve(dirname(markdown), decoded);
    if (!existsSync(resolved)) {
      failures.push(`${markdown}: missing link target ${target}`);
    }
  }
}

const cli = resolve(repositoryRoot, 'woml-cli/dist/cli.js');
if (!existsSync(cli)) {
  failures.push('woml-cli/dist/cli.js is missing; run `bun run build:js` first');
} else {
  const temporaryProject = mkdtempSync(resolve(tmpdir(), 'woml-docs-'));
  const temporaryExamples = resolve(temporaryProject, 'examples');
  try {
    cpSync(resolve(repositoryRoot, 'examples'), temporaryExamples, {
      recursive: true,
    });
    const result = Bun.spawnSync({
      cmd: [process.execPath, cli, 'check', temporaryExamples, '--json'],
      cwd: temporaryProject,
      stdout: 'pipe',
      stderr: 'pipe',
      env: { ...process.env, NO_COLOR: '1' },
    });
    if (result.exitCode !== 0) {
      failures.push(
        `examples/: woml check failed\n${result.stderr.toString().trim()}`
      );
    }
  } finally {
    rmSync(temporaryProject, { recursive: true, force: true });
  }
}

for (const example of filesUnder(resolve(repositoryRoot, 'examples'), '.woml')) {
  const source = readFileSync(example, 'utf8');
  for (const [label, pattern] of [
    ['personal sample value', /\b(?:Dali|Mohamed Ali)\b/u],
    ['machine-specific home path', /\/home\/[A-Za-z0-9._-]+\//u],
    ['pre-v1 workflow version', /version=["']0\.1["']/u],
  ] as const) {
    if (pattern.test(source)) failures.push(`${example}: contains ${label}`);
  }
}

const readme = readFileSync(resolve(repositoryRoot, 'README.md'), 'utf8');
if (!readme.includes('bun add --global woml-cli')) {
  failures.push('README.md: canonical install command is missing');
}
if (!readme.includes('npm install --global woml-cli')) {
  failures.push('README.md: npm install command is missing');
}
if (/bun add --global woml[^-\w]/u.test(readme)) {
  failures.push('README.md: obsolete unscoped woml package name is present');
}

const requiredFiles = [
  'docs/README.md',
  'docs/getting-started.md',
  'docs/cli-reference.md',
  'docs/language-reference.md',
  'examples/README.md',
  'SECURITY.md',
  'SUPPORT.md',
];
for (const file of requiredFiles) {
  const path = resolve(repositoryRoot, file);
  if (!existsSync(path) || statSync(path).size === 0) {
    failures.push(`${file}: required public documentation is missing or empty`);
  }
}

if (failures.length > 0) {
  throw new Error(`Documentation verification failed:\n- ${failures.join('\n- ')}`);
}

process.stdout.write(
  '[documentation] links, public guides, install identity, and every WOML example passed\n'
);
