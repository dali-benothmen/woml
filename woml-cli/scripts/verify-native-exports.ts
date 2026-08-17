import { copyFile, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, join, resolve } from 'node:path';
import { createRequire } from 'node:module';

export const expectedNativeExports = [
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

export function defaultStagedNativePath(): string {
  return resolve(
    import.meta.dir,
    '../dist',
    `woml-core.${process.platform}-${process.arch}.node`,
  );
}

export async function verifyNativeExports(
  source = defaultStagedNativePath()
): Promise<number> {
  const temporaryDirectory = await mkdtemp(
    join(tmpdir(), 'woml-native-exports-')
  );
  const stagedAddon = join(
    temporaryDirectory,
    `${basename(source).replace(/\.(so|dylib|dll|node)$/u, '')}.node`,
  );

  try {
    await copyFile(source, stagedAddon);
    const require = createRequire(import.meta.url);
    const addon = require(stagedAddon) as Record<string, unknown>;
    const actual = Object.keys(addon).sort();
    const expected = [...expectedNativeExports].sort();

    if (JSON.stringify(actual) !== JSON.stringify(expected)) {
      throw new Error(
        `WOML native exports changed.\nExpected: ${expected.join(', ')}\nActual: ${actual.join(', ')}`,
      );
    }

    return actual.length;
  } finally {
    try {
      await rm(temporaryDirectory, { recursive: true, force: true });
    } catch {
      // Windows holds file locks on loaded native addons after dlopen
    }
  }
}

if (import.meta.main) {
  const source = resolve(process.argv[2] ?? defaultStagedNativePath());
  const count = await verifyNativeExports(source);
  console.log(`Verified ${count} WOML native exports from ${source}.`);
}
