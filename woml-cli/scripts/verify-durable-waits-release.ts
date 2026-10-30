import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { readRustModuleSource } from './rust-source';

const repositoryRoot = resolve(import.meta.dir, '../..');
const required = [
  'docs/woml-durable-waits.md',
  'docs/protocols/durable-waits-v1.md',
  'docs/schemas/wait-inspection.v1.schema.json',
  'docs/schemas/run-inspection.v8.schema.json',
  'examples/durableWaits/README.md',
  'examples/durableWaits/timer.woml',
  'examples/durableWaits/ciCallback.woml',
  'examples/durableWaits/securityScan.woml',
  'examples/durableWaits/maintenanceWindow.woml',
  'examples/durableWaits/performance-budgets.v1.json',
  'skills/woml/references/tags.md',
  'skills/woml/references/cli.md',
  'woml-vscode/snippets/woml.code-snippets',
];

const sources = new Map<string, string>();
for (const relative of required) {
  const source = await readFile(resolve(repositoryRoot, relative), 'utf8');
  if (source.trim().length === 0) throw new Error(`${relative} is empty.`);
  sources.set(relative, source);
}

const budgets = JSON.parse(
  sources.get('examples/durableWaits/performance-budgets.v1.json')!
) as Record<string, unknown>;
if (
  budgets.profile !== 'woml.durable-waits-performance-budgets/v1' ||
  Object.entries(budgets).some(
    ([name, value]) =>
      name !== 'profile' && (!Number.isFinite(value) || Number(value) <= 0)
  )
) {
  throw new Error(
    'Durable Waits performance budgets are invalid or unversioned.'
  );
}

for (const schemaName of [
  'wait-inspection.v1.schema.json',
  'run-inspection.v8.schema.json',
]) {
  const schema = JSON.parse(sources.get(`docs/schemas/${schemaName}`)!) as {
    readonly 'x-status'?: string;
    readonly 'x-forbiddenFields'?: readonly string[];
  };
  if (
    schema['x-status'] !== 'durable-waits-dw7-frozen' ||
    !schema['x-forbiddenFields']?.includes('payload') ||
    !schema['x-forbiddenFields']?.includes('secret')
  ) {
    throw new Error(
      `${schemaName} does not freeze the payload-free DW9 boundary.`
    );
  }
}

const implementation = await Promise.all([
  readRustModuleSource(resolve(repositoryRoot, 'core/woml-engine/src/durable.rs')),
  readFile(resolve(repositoryRoot, 'core/woml-engine/src/webhook.rs'), 'utf8'),
  readFile(resolve(repositoryRoot, 'core/woml-engine/src/backup.rs'), 'utf8'),
  readFile(resolve(repositoryRoot, 'woml-cli/src/cli.ts'), 'utf8'),
]);
const joined = implementation.join('\n');
for (const marker of [
  'woml.external-signal-admission/v1',
  'woml.signal-result/v1',
  'WOML_SIGNAL_RATE_LIMITED',
  'WOML_SIGNAL_PAYLOAD_SECRET',
  'WOML_SIGNAL_PAYLOAD_TOO_LARGE',
  'woml.wait-inspection/v1',
  'woml.run-inspection/v8',
  'SUPPORTED_BACKUP_STORE_VERSIONS: [u32; 8] = [13, 14, 15, 16, 17, 18, 19, 20]',
  'Usage: woml signal <name>',
]) {
  if (!joined.includes(marker)) {
    throw new Error(`DW9 release marker is missing: ${marker}`);
  }
}

const snippets = sources.get('woml-vscode/snippets/woml.code-snippets')!;
for (const prefix of ['woml-wait', 'woml-wait-until', 'woml-wait-for']) {
  if (!snippets.includes(`"prefix": "${prefix}"`)) {
    throw new Error(`VS Code durable-wait snippet is missing: ${prefix}`);
  }
}

console.log(
  `Durable Waits release gate passed: ${required.length} public artifacts, ` +
    'Store v20 backup compatibility, bounded ingress, payload-free inspection, and editor/agent surfaces verified.'
);
