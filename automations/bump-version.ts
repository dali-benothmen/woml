#!/usr/bin/env bun
/**
 * WOML version bump tool.
 *
 * Edits every place the WOML version lives, regenerates Cargo.lock,
 * and runs the full release validation suite. After it finishes it
 * prints the exact git commit / tag / push commands for the human.
 *
 * It deliberately does NOT commit, tag, or push. Those are irreversible
 * and stay as deliberate human actions.
 *
 * Usage:
 *   bun automations/bump-version.ts --from 1.0.4 --to 1.0.5
 *   bun automations/bump-version.ts --from 1.0.4 --to 1.0.5 --dry-run
 */

import { readFile, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';

interface BumpArgs {
  readonly from: string;
  readonly to: string;
  readonly dryRun: boolean;
}

interface EditPlan {
  readonly path: string;
  readonly description: string;
  apply(): Promise<boolean>;
}

const SEMVER_RE = /^\d+\.\d+\.\d+$/u;

function parseSemver(value: string): readonly [number, number, number] {
  const parts = value.split('.').map(part => Number.parseInt(part, 10));
  if (parts.length !== 3 || parts.some(n => Number.isNaN(n) || n < 0)) {
    throw new Error(`Invalid semver: ${value}`);
  }
  return [parts[0]!, parts[1]!, parts[2]!] as const;
}

function isGreater(a: readonly [number, number, number], b: readonly [number, number, number]): boolean {
  for (let i = 0; i < 3; i++) {
    if (a[i]! > b[i]!) return true;
    if (a[i]! < b[i]!) return false;
  }
  return false;
}

function parseArgs(argv: readonly string[]): BumpArgs {
  let from: string | undefined;
  let to: string | undefined;
  let dryRun = false;
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === '--from') from = argv[++i];
    else if (arg === '--to') to = argv[++i];
    else if (arg === '--dry-run') dryRun = true;
    else throw new Error(`Unknown argument: ${arg}`);
  }
  if (!from || !to) {
    throw new Error('Both --from and --to are required. Example: --from 1.0.4 --to 1.0.5');
  }
  if (!SEMVER_RE.test(from) || !SEMVER_RE.test(to)) {
    throw new Error(`--from and --to must be semver (X.Y.Z). Got: --from ${from} --to ${to}`);
  }
  if (!isGreater(parseSemver(to), parseSemver(from))) {
    throw new Error(`--to (${to}) must be greater than --from (${from}).`);
  }
  return { from, to, dryRun };
}

async function findRepoRoot(startDir: string): Promise<string> {
  let dir = resolve(startDir);
  for (let i = 0; i < 16; i++) {
    try {
      const pkg = JSON.parse(await readFile(resolve(dir, 'package.json'), 'utf8')) as { readonly name?: string };
      if (pkg.name === 'woml-repository') return dir;
    } catch {
      // ignore
    }
    const parent = dirname(dir);
    if (parent === dir) {
      throw new Error(`Could not find woml-repository root from ${startDir}. Run from inside the repo.`);
    }
    dir = parent;
  }
  throw new Error(`Could not find woml-repository root after 16 levels. Run from inside the repo.`);
}

async function runCommand(cmd: readonly string[], cwd: string): Promise<{ readonly exitCode: number; readonly stdout: string; readonly stderr: string }> {
  const result = Bun.spawnSync({
    cmd: [...cmd],
    cwd,
    stdout: 'pipe',
    stderr: 'pipe',
  });
  return {
    exitCode: result.exitCode,
    stdout: result.stdout.toString(),
    stderr: result.stderr.toString(),
  };
}

async function checkCleanTree(repoRoot: string): Promise<void> {
  const { stdout, exitCode } = await runCommand(['git', 'status', '--porcelain'], repoRoot);
  if (exitCode !== 0) {
    throw new Error(`git status failed (exit ${exitCode}): ${stdout}`);
  }
  if (stdout.trim().length > 0) {
    throw new Error(
      `Git tree is dirty. Commit or stash unrelated changes before bumping.\n${stdout}`,
    );
  }
}

function buildEditPlan(
  repoRoot: string,
  from: string,
  to: string,
  dryRun: boolean,
): readonly EditPlan[] {
  const plans: EditPlan[] = [];

  const addPackageJson = (relativePath: string, description: string): void => {
    const fullPath = resolve(repoRoot, relativePath);
    plans.push({
      path: fullPath,
      description,
      async apply() {
        const source = await readFile(fullPath, 'utf8');
        const updated = source.replace(`"version": "${from}"`, `"version": "${to}"`);
        if (updated === source) {
          throw new Error(
            `${relativePath}: expected to find "version": "${from}" but did not. ` +
              `The manifest may already be at ${to} or the version field was renamed.`,
          );
        }
        if (!dryRun) await writeFile(fullPath, updated, 'utf8');
        return updated !== source;
      },
    });
  };

  const addCargoToml = (relativePath: string, description: string, multiEdit: boolean): void => {
    const fullPath = resolve(repoRoot, relativePath);
    plans.push({
      path: fullPath,
      description,
      async apply() {
        let source = await readFile(fullPath, 'utf8');
        const packageRe = new RegExp(`version = "${from}"`, 'g');
        const depRe = new RegExp(`woml-engine = \\{ version = "${from}"`, 'g');
        const before = source;
        source = source.replace(packageRe, `version = "${to}"`);
        if (multiEdit) source = source.replace(depRe, `woml-engine = { version = "${to}"`);
        if (source === before) {
          throw new Error(
            `${relativePath}: expected to find version = "${from}" but did not. ` +
              `The manifest may already be at ${to} or the version was renamed.`,
          );
        }
        if (!dryRun) await writeFile(fullPath, source, 'utf8');
        return source !== before;
      },
    });
  };

  const addReplaceAll = (relativePath: string, description: string): void => {
    const fullPath = resolve(repoRoot, relativePath);
    plans.push({
      path: fullPath,
      description,
      async apply() {
        const source = await readFile(fullPath, 'utf8');
        const updated = source.replaceAll(from, to);
        if (updated === source) {
          throw new Error(
            `${relativePath}: expected to find "${from}" but did not. ` +
              `The script may already be at ${to} or the string was renamed.`,
          );
        }
        if (!dryRun) await writeFile(fullPath, updated, 'utf8');
        return updated !== source;
      },
    });
  };

  const addDockerfile = (relativePath: string, description: string): void => {
    const fullPath = resolve(repoRoot, relativePath);
    plans.push({
      path: fullPath,
      description,
      async apply() {
        const source = await readFile(fullPath, 'utf8');
        const updated = source.replace(`ARG WOML_VERSION=${from}`, `ARG WOML_VERSION=${to}`);
        if (updated === source) {
          throw new Error(`${relativePath}: expected ARG WOML_VERSION=${from} but did not.`);
        }
        if (!dryRun) await writeFile(fullPath, updated, 'utf8');
        return updated !== source;
      },
    });
  };

  const addRustTest = (relativePath: string, description: string): void => {
    const fullPath = resolve(repoRoot, relativePath);
    plans.push({
      path: fullPath,
      description,
      async apply() {
        const source = await readFile(fullPath, 'utf8');
        const updated = source.replace(`\\"${from}\\"`, `\\"${to}\\"`);
        if (updated === source) {
          throw new Error(`${relativePath}: expected escaped \\"${from}\\" but did not.`);
        }
        if (!dryRun) await writeFile(fullPath, updated, 'utf8');
        return updated !== source;
      },
    });
  };

  addPackageJson('woml-cli/package.json', 'public woml-cli version');
  addPackageJson('woml/package.json', 'private @woml/compiler version');
  addPackageJson('package.json', 'private woml-repository version');
  addPackageJson('woml-vscode/package.json', 'private woml-language version');
  addCargoToml('core/woml-engine/Cargo.toml', 'woml-engine crate version', false);
  addCargoToml('core/woml-native/Cargo.toml', 'woml-native crate version + woml-engine dep', true);
  addDockerfile('examples/production/deployment/Dockerfile', 'Dockerfile ARG WOML_VERSION');
  addReplaceAll('woml-cli/scripts/release-package.ts', 'identity check + log line');
  addReplaceAll('woml-cli/scripts/verify-production-release.ts', 'identity check');
  addReplaceAll('woml-cli/scripts/verify-final-release-review.ts', 'identity check');
  addReplaceAll('woml-cli/tests/release-identity.test.ts', 'release-identity test fixtures');
  addReplaceAll('woml-cli/tests/release-family.test.ts', 'release-family test fixtures');
  addReplaceAll('woml-cli/tests/native-platform-release.test.ts', 'native-platform-release test fixture');
  addRustTest('core/woml-native/tests/separation.rs', 'separation.rs version string');

  return plans;
}

async function applyEditPlans(plans: readonly EditPlan[], repoRoot: string): Promise<void> {
  for (const plan of plans) {
    const changed = await plan.apply();
    const relativePath = plan.path.slice(repoRoot.length + 1);
    process.stdout.write(
      changed
        ? `  ${dryRunLabel()} ${relativePath}  (${plan.description})\n`
        : `  ${dryRunLabel()} ${relativePath}  (no change needed)\n`,
    );
  }
}

function dryRunLabel(): string {
  return '[edit]';
}

async function regenerateCargoLock(repoRoot: string, dryRun: boolean): Promise<void> {
  if (dryRun) {
    process.stdout.write('  [skip] cargo update --workspace (dry-run)\n');
    return;
  }
  const result = await runCommand(
    ['cargo', 'update', '--workspace', '--manifest-path', 'core/Cargo.toml'],
    repoRoot,
  );
  if (result.exitCode !== 0) {
    throw new Error(`cargo update failed (exit ${result.exitCode}):\n${result.stderr}`);
  }
}

async function runValidation(repoRoot: string): Promise<void> {
  const checks: ReadonlyArray<{ readonly label: string; readonly cmd: readonly string[]; readonly cwd: string }> = [
    {
      label: 'release-identity + native-platform-release',
      cmd: [
        'bun',
        'test',
        'tests/release-identity.test.ts',
        'tests/native-platform-release.test.ts',
        '--max-concurrency=1',
      ],
      cwd: resolve(repoRoot, 'woml-cli'),
    },
    {
      label: 'release-automation + release-artifact + release-family',
      cmd: [
        'bun',
        'test',
        'tests/release-automation.test.ts',
        'tests/release-artifact.test.ts',
        'tests/release-family.test.ts',
        '--max-concurrency=1',
      ],
      cwd: resolve(repoRoot, 'woml-cli'),
    },
    {
      label: 'verify-native-platform-release',
      cmd: ['bun', 'scripts/verify-native-platform-release.ts'],
      cwd: resolve(repoRoot, 'woml-cli'),
    },
    {
      label: 'verify-documentation',
      cmd: ['bun', 'scripts/verify-documentation.ts'],
      cwd: resolve(repoRoot, 'woml-cli'),
    },
    {
      label: 'typecheck',
      cmd: ['bun', 'run', 'typecheck'],
      cwd: resolve(repoRoot, 'woml-cli'),
    },
  ];
  for (const check of checks) {
    process.stdout.write(`  [check] ${check.label}... `);
    const result = await runCommand(check.cmd, check.cwd);
    if (result.exitCode !== 0) {
      process.stdout.write('FAIL\n');
      throw new Error(
        `Validation step "${check.label}" failed (exit ${result.exitCode}).\n` +
          `Command: ${check.cmd.join(' ')}\n` +
          `Cwd: ${check.cwd}\n\n` +
          `${result.stderr || result.stdout}`,
      );
    }
    process.stdout.write('ok\n');
  }
}

async function showDiffSummary(repoRoot: string): Promise<void> {
  const result = await runCommand(['git', 'diff', '--stat'], repoRoot);
  process.stdout.write('\n--- Diff summary ---\n');
  process.stdout.write(result.stdout);
  process.stdout.write('--------------------\n\n');
}

function printNextSteps(to: string): void {
  process.stdout.write(
    [
      '',
      'Suggested follow-up commands:',
      '',
      '  git add -A',
      `  git commit -m "chore: bump to ${to}"`,
      `  git tag v${to}`,
      '  git push origin master',
      `  git push --force origin v${to}`,
      '',
      'After CI uploads the woml-release-family artifact, publish manually:',
      '',
      '  for dir in woml-release-family/release/platforms/native-*; do',
      '    (cd "$dir" && npm publish --access public)',
      '  done',
      '  (cd woml-release-family/release/main && npm publish --access public)',
      '',
      'This tool does NOT run those commands. They are irreversible.',
      '',
    ].join('\n'),
  );
}

async function main(): Promise<void> {
  const args = parseArgs(process.argv.slice(2));
  const repoRoot = await findRepoRoot(process.cwd());

  process.stdout.write(`Bumping ${args.from} -> ${args.to}\n`);
  process.stdout.write(`Repo root: ${repoRoot}\n`);
  if (args.dryRun) process.stdout.write('Dry run: no files will be edited, cargo lock will not be regenerated.\n');
  process.stdout.write('\n');

  await checkCleanTree(repoRoot);
  process.stdout.write('Git tree is clean.\n\n');

  process.stdout.write('Edits:\n');
  const plans = buildEditPlan(repoRoot, args.from, args.to, args.dryRun);
  await applyEditPlans(plans, repoRoot);
  process.stdout.write('\n');

  await regenerateCargoLock(repoRoot, args.dryRun);

  if (args.dryRun) {
    process.stdout.write('\nDry run complete. Re-run without --dry-run to apply edits.\n');
    return;
  }

  process.stdout.write('\nValidation:\n');
  await runValidation(repoRoot);
  process.stdout.write('\nAll validation passed.\n');

  await showDiffSummary(repoRoot);
  printNextSteps(args.to);
}

if (import.meta.main) {
  await main();
}
