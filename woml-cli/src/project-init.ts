import { lstat, mkdir, readFile, rmdir, unlink, writeFile } from 'node:fs/promises';
import { basename, resolve } from 'node:path';

const TEMPLATE_ROOT = resolve(import.meta.dir, '../templates');

export interface InitializedWomlProjectV1 {
  readonly profile: 'woml.project-initialization/v1';
  readonly status: 'created';
  readonly directory: string;
  readonly manifestPath: string;
  readonly workflowPath: string;
  readonly authoringGuidePath: string;
}

export class WomlProjectInitializationError extends Error {
  readonly code: string;

  constructor(code: string, message: string) {
    super(message);
    this.name = 'WomlProjectInitializationError';
    this.code = code;
  }
}

function projectName(directory: string): string {
  const normalized = basename(directory)
    .normalize('NFC')
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/gu, '-')
    .replace(/^[^a-z]+/u, '')
    .replace(/[._-]+$/u, '')
    .slice(0, 128);
  return normalized.length === 0 ? 'woml-project' : normalized;
}

async function exists(path: string): Promise<boolean> {
  return (await lstat(path).catch(() => undefined)) !== undefined;
}

/** Creates only the reviewed local template and rolls back files it created on failure. */
export async function initializeWomlProject(
  directory: string
): Promise<InitializedWomlProjectV1> {
  const root = resolve(directory);
  const manifestPath = resolve(root, 'woml.project.json');
  const workflowsDirectory = resolve(root, 'workflows');
  const workflowPath = resolve(workflowsDirectory, 'hello.woml');
  const authoringGuidePath = resolve(root, 'AGENTS.md');
  const collisions = (
    await Promise.all(
      [manifestPath, workflowPath, authoringGuidePath].map(async path => ({
        path,
        exists: await exists(path),
      }))
    )
  ).filter(item => item.exists);
  if (collisions.length > 0) {
    throw new WomlProjectInitializationError(
      'WOML_PROJECT_INIT_COLLISION',
      `Refusing to overwrite existing project file${collisions.length === 1 ? '' : 's'}: ${collisions
        .map(item => item.path)
        .join(', ')}.`
    );
  }

  const rootExisted = await exists(root);
  const workflowsExisted = await exists(workflowsDirectory);
  const created: string[] = [];
  try {
    await mkdir(workflowsDirectory, { recursive: true, mode: 0o755 });
    const [manifestTemplate, workflowTemplate, authoringGuideTemplate] = await Promise.all([
      readFile(resolve(TEMPLATE_ROOT, 'woml.project.json'), 'utf8'),
      readFile(resolve(TEMPLATE_ROOT, 'workflows/hello.woml'), 'utf8'),
      readFile(resolve(TEMPLATE_ROOT, 'AGENTS.md'), 'utf8'),
    ]);
    await writeFile(
      manifestPath,
      manifestTemplate.replaceAll('__PROJECT_NAME__', projectName(root)),
      { encoding: 'utf8', mode: 0o644, flag: 'wx' }
    );
    created.push(manifestPath);
    await writeFile(workflowPath, workflowTemplate, {
      encoding: 'utf8',
      mode: 0o644,
      flag: 'wx',
    });
    created.push(workflowPath);
    await writeFile(authoringGuidePath, authoringGuideTemplate, {
      encoding: 'utf8',
      mode: 0o644,
      flag: 'wx',
    });
    created.push(authoringGuidePath);
  } catch (error) {
    await Promise.all(created.map(path => unlink(path).catch(() => {})));
    if (!workflowsExisted) await rmdir(workflowsDirectory).catch(() => {});
    if (!rootExisted) await rmdir(root).catch(() => {});
    if (
      error instanceof Error &&
      'code' in error &&
      (error.code === 'EEXIST' || error.code === 'EISDIR')
    ) {
      throw new WomlProjectInitializationError(
        'WOML_PROJECT_INIT_COLLISION',
        'Refusing to overwrite a project file created concurrently.'
      );
    }
    throw error;
  }

  return {
    profile: 'woml.project-initialization/v1',
    status: 'created',
    directory: root,
    manifestPath,
    workflowPath,
    authoringGuidePath,
  };
}

