import { copyFile, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, join, resolve } from 'node:path';
import { createRequire } from 'node:module';

const expectedExports = [
  'activateWomlWebhookRuntime',
  'cancelWomlRun',
  'createWomlBackup',
  'executeWomlRetention',
  'executeWomlRetentionAsync',
  'executeWomlWorkflow',
  'executeWomlWorkflowDurable',
  'executeWomlWorkflowDurableOutcome',
  'executeWomlWorkflowDurableOutcomeWithProgress',
  'executeWomlWorkflowDurableWithProgress',
  'hasWomlWorkflowDefinition',
  'inspectWomlBackupStore',
  'inspectWomlRun',
  'inspectWomlRunPresentation',
  'inspectWomlRunV2',
  'inspectWomlStoredRunRequirements',
  'listWomlRunPresentations',
  'listWomlRuns',
  'observeWomlRuntime',
  'planWomlRetention',
  'prepareWomlRestoredStore',
  'readWomlLastRetentionResult',
  'recordWomlVerifiedBackup',
  'recoverWomlRuns',
  'resolveWomlApproval',
  'resolveWomlNotificationApproval',
  'resumeWomlStoredRunWithProgress',
  'resumeWomlWorkflowDurableOutcome',
  'resumeWomlWorkflowDurableOutcomeWithProgress',
  'resumeWomlWorkflowDurableWithProgress',
  'runWomlNotificationProviderJourney',
  'settleWomlApprovalTimeout',
  'startWomlWebhookRuntime',
  'stopWomlWebhookRuntime',
  'submitWomlManualTrigger',
  'submitWomlTriggerOccurrence',
] as const;

function defaultLibraryName(): string {
  if (process.platform === 'win32') return 'woml_core.dll';
  if (process.platform === 'darwin') return 'libwoml_core.dylib';
  if (process.platform === 'linux') return 'libwoml_core.so';
  throw new Error(`Unsupported native platform: ${process.platform}.`);
}

const source = resolve(
  process.argv[2] ??
    resolve(import.meta.dir, '../../dist/target/debug', defaultLibraryName()),
);
const temporaryDirectory = await mkdtemp(join(tmpdir(), 'woml-native-exports-'));
const stagedAddon = join(
  temporaryDirectory,
  `${basename(source).replace(/\.(so|dylib|dll)$/u, '')}.node`,
);

try {
  await copyFile(source, stagedAddon);
  const require = createRequire(import.meta.url);
  const addon = require(stagedAddon) as Record<string, unknown>;
  const actual = Object.keys(addon).sort();
  const expected = [...expectedExports].sort();

  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(
      `WOML native exports changed.\nExpected: ${expected.join(', ')}\nActual: ${actual.join(', ')}`,
    );
  }

  console.log(`Verified ${actual.length} WOML native exports from ${source}.`);
} finally {
  await rm(temporaryDirectory, { recursive: true, force: true });
}
