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
  'exportWomlArtifact',
  'exportWomlArtifactBackup',
  'hasWomlWorkflowDefinition',
  'inspectWomlArtifact',
  'inspectWomlBackupStore',
  'inspectWomlCorrectionV1',
  'inspectWomlRun',
  'inspectWomlRunPresentation',
  'inspectWomlRunV2',
  'inspectWomlWaitsV1',
  'inspectWomlLocksV1',
  'inspectWomlOperationalProgress',
  'inspectWomlRemoteDispatch',
  'observeWomlLocksV1',
  'observeWomlOperationalOutput',
  'inspectWomlRunArtifacts',
  'inspectWomlStoredRunRequirements',
  'listWomlRunPresentations',
  'listWomlRuns',
  'maintainWomlArtifacts',
  'observeWomlArtifacts',
  'observeWomlCorrectionsV1',
  'observeWomlRuntime',
  'planWomlOperationalOutputPrune',
  'planWomlRetention',
  'prepareWomlRestoredStore',
  'readWomlLastRetentionResult',
  'readWomlOperationalOutputPage',
  'reconcileWomlRemoteDispatch',
  'recordWomlVerifiedBackup',
  'pruneWomlOperationalOutput',
  'recoverWomlCorrection',
  'recoverWomlRuns',
  'resolveWomlApproval',
  'resolveWomlNotificationApproval',
  'resolveWomlRichApproval',
  'restoreWomlArtifactBackup',
  'resumeWomlStoredRunWithProgress',
  'resumeWomlWorkflowDurableOutcome',
  'resumeWomlWorkflowDurableOutcomeWithProgress',
  'resumeWomlWorkflowDurableWithProgress',
  'runWomlNotificationProviderJourney',
  'settleWomlApprovalTimeout',
  'settleWomlTimerWait',
  'startWomlWebhookRuntime',
  'stopWomlWebhookRuntime',
  'submitWomlManualTrigger',
  'submitWomlTriggerOccurrence',
  'submitWomlExternalSignal',
  'verifyWomlArtifactBackup',
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
