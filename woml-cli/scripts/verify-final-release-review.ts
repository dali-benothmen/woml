#!/usr/bin/env bun

import { createHash } from 'node:crypto';
import { mkdtemp, readFile, readdir, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, relative, resolve } from 'node:path';

import { prepareMainPackage } from './native-release';
import { verifyMainPackage } from './release-package';

const repositoryRoot = resolve(import.meta.dir, '../..');

const credentialPatterns = [
  /\bxox[baprs]-[A-Za-z0-9-]{20,}\b/gu,
  /\bxapp-[A-Za-z0-9-]{20,}\b/gu,
  /\bgh[pousr]_[A-Za-z0-9]{20,}\b/gu,
  /\bgithub_pat_[A-Za-z0-9_]{20,}\b/gu,
  /\bsk-(?:proj-)?[A-Za-z0-9_-]{24,}\b/gu,
  /\bAKIA[0-9A-Z]{16}\b/gu,
  /\b[0-9]{8,10}:[A-Za-z0-9_-]{30,}\b/gu,
  /https?:\/\/[^/\s:@]+:[^/@\s]+@/gu,
  /-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----/gu,
] as const;

const machineIdentityPatterns = [
  /\/home\/(?!runner(?:\/|\b))[^/\s"']+\//gu,
  /\b[A-Za-z]:\\Users\\[^\\\s"']+\\/gu,
  /\bT0BNLAP0DRT\b/gu,
] as const;

const textExtensions = new Set([
  '', '.css', '.dockerignore', '.html', '.js', '.json', '.lock', '.md',
  '.mjs', '.rc', '.rs', '.sh', '.svg', '.toml', '.ts', '.tsx', '.txt',
  '.woml', '.yaml', '.yml',
]);

function extension(path: string): string {
  const name = path.slice(path.lastIndexOf('/') + 1);
  const index = name.lastIndexOf('.');
  return index < 0 ? '' : name.slice(index).toLowerCase();
}

function lineAt(source: string, index: number): number {
  return source.slice(0, index).split('\n').length;
}

export interface SensitiveFinding {
  readonly path: string;
  readonly line: number;
  readonly kind: 'credential' | 'machine-identity';
}

export function scanSensitiveText(path: string, source: string): SensitiveFinding[] {
  const findings: SensitiveFinding[] = [];
  for (const pattern of credentialPatterns) {
    pattern.lastIndex = 0;
    for (const match of source.matchAll(pattern)) {
      const syntheticFixture =
        /(?:^|\/)(?:tests?|fixtures?)(?:\/|$)/u.test(path) &&
        /(?:test|fake|secret|password|must-not|clean-package|super-secret|n\d+|timeout|(?:bot|app)-value)/iu.test(match[0]);
      if (!syntheticFixture) {
        findings.push({
          path,
          line: lineAt(source, match.index ?? 0),
          kind: 'credential',
        });
      }
    }
  }
  for (const pattern of machineIdentityPatterns) {
    pattern.lastIndex = 0;
    for (const match of source.matchAll(pattern)) {
      findings.push({
        path,
        line: lineAt(source, match.index ?? 0),
        kind: 'machine-identity',
      });
    }
  }
  return findings;
}

async function filesUnder(root: string): Promise<string[]> {
  const result: string[] = [];
  const visit = async (directory: string): Promise<void> => {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) await visit(path);
      else if (entry.isFile()) result.push(relative(root, path).replaceAll('\\', '/'));
    }
  };
  await visit(root);
  return result.sort();
}

async function releaseSourceFiles(): Promise<string[]> {
  const result = Bun.spawnSync({
    cmd: ['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard'],
    cwd: repositoryRoot,
    stdout: 'pipe',
    stderr: 'pipe',
  });
  if (result.exitCode !== 0) {
    throw new Error(`Could not list release source files: ${result.stderr.toString().trim()}`);
  }
  return result.stdout.toString().split('\0').filter(Boolean);
}

async function scanFiles(root: string, paths: readonly string[]): Promise<void> {
  const findings: SensitiveFinding[] = [];
  for (const path of paths) {
    if (!textExtensions.has(extension(path))) continue;
    const absolute = resolve(root, path);
    if ((await stat(absolute)).size > 20 * 1024 * 1024) continue;
    findings.push(...scanSensitiveText(path, await readFile(absolute, 'utf8')));
  }
  if (findings.length > 0) {
    throw new Error(
      `Sensitive release content detected:\n${findings.map(item => `- ${item.path}:${item.line} (${item.kind})`).join('\n')}`,
    );
  }
}

async function assertContains(path: string, values: readonly string[]): Promise<void> {
  const source = await readFile(resolve(repositoryRoot, path), 'utf8');
  for (const value of values) {
    if (!source.includes(value)) throw new Error(`${path} is missing ${JSON.stringify(value)}.`);
  }
}

async function verifyLegalAndProductMetadata(): Promise<void> {
  const manifest = JSON.parse(
    await readFile(resolve(repositoryRoot, 'woml-cli/package.json'), 'utf8'),
  ) as Record<string, unknown>;
  if (
    manifest.name !== '@woml-org/woml' ||
    manifest.version !== '1.0.0' ||
    manifest.license !== 'Apache-2.0' ||
    manifest.author !== 'Mohamed Ali Ben Othmen'
  ) {
    throw new Error('The public package identity, license, version, or author is not frozen.');
  }
  const metadata = JSON.stringify(manifest);
  for (const value of [
    'github.com/dali-benothmen/woml.git',
    'github.com/dali-benothmen/woml#readme',
    'github.com/dali-benothmen/woml/issues',
    'NOTICE.md',
  ]) {
    if (!metadata.includes(value)) throw new Error(`Public package metadata is missing ${value}.`);
  }
  await assertContains('LICENSE', ['Apache License', 'Copyright 2025-2026 Mohamed Ali Ben Othmen']);
  await assertContains('NOTICE.md', ['WOML', 'core/Cargo.lock', 'woml-cli/bun.lock']);
  if (
    (await readFile(resolve(repositoryRoot, 'NOTICE.md'), 'utf8')) !==
    (await readFile(resolve(repositoryRoot, 'woml-cli/NOTICE.md'), 'utf8'))
  ) {
    throw new Error('The source-package NOTICE.md differs from the repository notice.');
  }
  await assertContains('SECURITY.md', ['Report a vulnerability', 'docs/woml-data-security.md']);
  await assertContains('SUPPORT.md', ['GitHub Discussions', 'GitHub Issues', 'SECURITY.md']);
  await assertContains('.github/workflows/security.yml', [
    'bun audit --cwd woml-cli',
    'cargo-audit --version 0.22.2 --locked',
    'cargo audit --file core/Cargo.lock',
    'contents: read',
  ]);
}

async function verifySecurityDefaults(): Promise<void> {
  await assertContains('woml-cli/src/runtime-config.ts', [
    "'127.0.0.1'",
    "!['127.0.0.1', 'localhost', '::1'].includes(adminHost)",
  ]);
  await assertContains('woml-cli/src/runtime-control.ts', [
    'timingSafeEqual',
    'mode: 0o600',
    "const LOOPBACK_HOSTS = new Set(['127.0.0.1', 'localhost', '::1'])",
  ]);
  await assertContains('woml-cli/src/secrets/bun-secret-store.ts', [
    'allowUnrestrictedAccess: false',
  ]);
  await assertContains('woml-cli/src/secrets/mounted-file-secret-store.ts', [
    '(entry.mode & 0o077) !== 0',
    '(entry.mode & 0o022) !== 0',
  ]);
  await assertContains('.gitignore', ['.woml/', '*.db', '*.sqlite', '*.log', '.env']);
  await assertContains('docs/woml-data-security.md', [
    'not an encryption boundary',
    'hostile multi-tenant sandbox',
  ]);
}

async function packageDigest(root: string): Promise<string> {
  const hash = createHash('sha256');
  for (const path of await filesUnder(root)) {
    hash.update(path);
    hash.update('\0');
    hash.update(await readFile(resolve(root, path)));
  }
  return hash.digest('hex');
}

export async function verifyFinalReleaseReview(): Promise<{
  readonly sourceFiles: number;
  readonly packageFiles: number;
  readonly packageDigest: string;
}> {
  const sources = await releaseSourceFiles();
  await scanFiles(repositoryRoot, sources);
  await verifyLegalAndProductMetadata();
  await verifySecurityDefaults();

  const temporary = await mkdtemp(resolve(tmpdir(), 'woml-final-review-'));
  try {
    await prepareMainPackage(temporary);
    await verifyMainPackage(temporary);
    const packaged = await filesUnder(temporary);
    await scanFiles(temporary, packaged);
    return {
      sourceFiles: sources.length,
      packageFiles: packaged.length,
      packageDigest: await packageDigest(temporary),
    };
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}

if (import.meta.main) {
  const result = await verifyFinalReleaseReview();
  process.stdout.write(
    `[V1R8] reviewed ${result.sourceFiles} release source files and ${result.packageFiles} packaged files; staged content SHA-256 ${result.packageDigest}.\n`,
  );
}
