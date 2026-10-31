import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { readRustModuleSource } from './rust-source';

const repositoryRoot = resolve(import.meta.dir, '../..');
const required = [
  'docs/woml-locks-and-leases.md',
  'docs/woml-lock-fencing.md',
  'docs/woml-lock-threat-model.md',
  'docs/woml-lock-operations.md',
  'docs/protocols/locks-and-leases-v1.md',
  'docs/schemas/locks-and-leases.v1.schema.json',
  'docs/schemas/lock-inspection.v1.schema.json',
  'examples/locksAndLeases/README.md',
  'examples/locksAndLeases/deployment.woml',
  'examples/locksAndLeases/databaseMigration.woml',
  'examples/locksAndLeases/backupRestore.woml',
  'examples/locksAndLeases/credentialRotation.woml',
  'examples/locksAndLeases/cacheRebuild.woml',
  'examples/locksAndLeases/performance-budgets.v1.json',
  'skills/woml/references/tags.md',
  'skills/woml/references/services.md',
  'skills/woml/references/diagnostics.md',
  'woml-vscode/snippets/woml.code-snippets',
];

const sources = new Map<string, string>();
for (const relative of required) {
  const source = await readFile(resolve(repositoryRoot, relative), 'utf8');
  if (source.trim().length === 0) throw new Error(`${relative} is empty.`);
  sources.set(relative, source);
}

const budgets = JSON.parse(
  sources.get('examples/locksAndLeases/performance-budgets.v1.json')!
) as {
  readonly profile?: string;
  readonly budgets?: Record<string, number>;
};
if (
  budgets.profile !== 'woml.locks-and-leases-performance-budgets/v1' ||
  budgets.budgets === undefined ||
  Object.values(budgets.budgets).some(
    value => !Number.isFinite(value) || value <= 0
  ) ||
  budgets.budgets.maximumRecoveryBatch !== 250
) {
  throw new Error('Locks and Leases performance budgets are invalid.');
}

const contract = JSON.parse(
  await readFile(
    resolve(
      repositoryRoot,
      'woml/tests/fixtures/locks-and-leases/contracts.v1.json'
    ),
    'utf8'
  )
) as {
  readonly profile?: string;
  readonly modelVersion?: number;
  readonly eventVersion?: number;
  readonly storeVersion?: number;
  readonly limits?: {
    readonly waitersPerKey?: number;
    readonly waitingAcquisitionsPerStore?: number;
  };
};
if (
  contract.profile !== 'woml.locks-and-leases/v1' ||
  contract.modelVersion !== 18 ||
  contract.eventVersion !== 17 ||
  contract.storeVersion !== 16 ||
  contract.limits?.waitersPerKey !== 1024 ||
  contract.limits.waitingAcquisitionsPerStore !== 10_000
) {
  throw new Error('The frozen Locks and Leases v1 contract drifted.');
}

const implementation = await Promise.all([
  readRustModuleSource(resolve(repositoryRoot, 'core/woml-engine/src/durable.rs')),
  readFile(resolve(repositoryRoot, 'core/woml-engine/src/backup.rs'), 'utf8'),
  readFile(resolve(repositoryRoot, 'core/woml-engine/src/retention.rs'), 'utf8'),
  readFile(resolve(repositoryRoot, 'woml-cli/src/cli.ts'), 'utf8'),
]);
const joined = implementation.join('\n');
for (const marker of [
  'LOCK_WAITERS_PER_KEY_LIMIT: usize = 1_024',
  'LOCK_WAITERS_PER_STORE_LIMIT: usize = 10_000',
  'LOCK_RECOVERY_BATCH_LIMIT: usize = 250',
  'LockFencingExhausted',
  "UPDATE woml_lock_waiters SET status = 'lost' WHERE status = 'held'",
  "status IN ('waiting', 'held')",
  'Shows the run together with durable waits, artifacts, and lock/lease state.',
]) {
  if (!joined.includes(marker)) {
    throw new Error(`LL9 release marker is missing: ${marker}`);
  }
}

const snippets = sources.get('woml-vscode/snippets/woml.code-snippets')!;
if (!snippets.includes('"prefix": "woml-lock"')) {
  throw new Error('The VS Code lock snippet is missing.');
}
const skill = [
  sources.get('skills/woml/references/tags.md'),
  sources.get('skills/woml/references/services.md'),
  sources.get('skills/woml/references/diagnostics.md'),
].join('\n');
for (const marker of ['<lock>', 'services.locks.withLease()', 'WOML_LOCK_LEASE_LOST']) {
  if (!skill.includes(marker)) throw new Error(`AI skill marker is missing: ${marker}`);
}

console.log(
  `Locks and Leases release gate passed: ${required.length} public artifacts, ` +
    'Store v16 limits, restore/retention fencing safety, editor, AI, and CLI surfaces verified.'
);
