import { randomUUID } from 'node:crypto';
import { readdir } from 'node:fs/promises';
import { relative, resolve } from 'node:path';

interface Arguments {
  readonly mainRoot: string;
  readonly nativeRoot: string;
}

function argumentsFrom(values: readonly string[]): Arguments {
  const options = new Map<string, string>();
  for (let index = 0; index < values.length; index += 2) {
    const option = values[index];
    const value = values[index + 1];
    if (option === undefined || value === undefined) break;
    options.set(option, value);
  }
  const mainRoot = options.get('--main-root');
  const nativeRoot = options.get('--native-root');
  if (mainRoot === undefined || nativeRoot === undefined) {
    throw new Error(
      'Usage: smoke-docker-release-candidate.ts --main-root <path> --native-root <path>',
    );
  }
  return { mainRoot: resolve(mainRoot), nativeRoot: resolve(nativeRoot) };
}

async function archive(root: string): Promise<string> {
  const names = (await readdir(root)).filter(name => name.endsWith('.tgz'));
  if (names.length !== 1) {
    throw new Error(`Expected one npm archive in ${root}, found ${names.length}.`);
  }
  return resolve(root, names[0]!);
}

function docker(args: readonly string[], allowFailure = false): string {
  const result = Bun.spawnSync(['docker', ...args], {
    cwd: repositoryRoot,
    stdout: 'pipe',
    stderr: 'pipe',
  });
  if (!allowFailure && result.exitCode !== 0) {
    throw new Error(
      `docker ${args.join(' ')} failed:\n${result.stdout.toString()}${result.stderr.toString()}`,
    );
  }
  return result.stdout.toString().trim();
}

async function waitForPublicPort(container: string): Promise<number> {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    const mapping = docker(['port', container, '3000/tcp'], true);
    const match = mapping.match(/127\.0\.0\.1:(\d+)/u);
    if (match !== null) {
      const port = Number(match[1]);
      try {
        const response = await fetch(`http://127.0.0.1:${port}/container-smoke`, {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ value: 42 }),
        });
        if (response.status === 202) return port;
      } catch {
        // The container is not accepting public trigger traffic yet.
      }
    }
    await Bun.sleep(100);
  }
  throw new Error(`Container did not become ready:\n${docker(['logs', container], true)}`);
}

const repositoryRoot = resolve(import.meta.dir, '../..');
const args = argumentsFrom(process.argv.slice(2));
const mainArchive = relative(repositoryRoot, await archive(args.mainRoot));
const nativeArchive = relative(repositoryRoot, await archive(args.nativeRoot));
if (mainArchive.startsWith('..') || nativeArchive.startsWith('..')) {
  throw new Error('Docker candidate archives must be inside the repository build context.');
}

const suffix = randomUUID().replaceAll('-', '').slice(0, 12);
const image = `woml-release-candidate:${suffix}`;
const container = `woml-release-candidate-${suffix}`;
const volume = `woml-release-candidate-${suffix}`;

try {
  docker([
    'build',
    '--file',
    'woml-cli/tests/fixtures/release-candidate/Dockerfile',
    '--build-arg',
    `MAIN_ARCHIVE=${mainArchive}`,
    '--build-arg',
    `NATIVE_ARCHIVE=${nativeArchive}`,
    '--tag',
    image,
    '.',
  ]);
  docker(['volume', 'create', volume]);

  const start = (): void => {
    docker([
      'run',
      '--detach',
      '--name',
      container,
      '--publish',
      '127.0.0.1::3000',
      '--volume',
      `${volume}:/app/data`,
      image,
    ]);
  };
  start();
  await waitForPublicPort(container);
  if (docker(['exec', container, 'id', '-u']) === '0') {
    throw new Error('The WOML container is running as root.');
  }
  if (docker(['port', container]).includes('3001')) {
    throw new Error('The private Runtime Admin listener was published by Docker.');
  }
  const firstRuns = JSON.parse(
    docker([
      'exec',
      container,
      'woml',
      'list',
      '--state',
      '/app/data/state.sqlite',
      '--json',
    ]),
  ) as { readonly runs: readonly unknown[] };
  if (firstRuns.runs.length < 1) {
    throw new Error('The first container did not persist its accepted workflow run.');
  }

  docker(['stop', '--time', '15', container]);
  docker(['rm', container]);
  start();
  await waitForPublicPort(container);
  const restartedRuns = JSON.parse(
    docker([
      'exec',
      container,
      'woml',
      'list',
      '--state',
      '/app/data/state.sqlite',
      '--json',
    ]),
  ) as { readonly runs: readonly unknown[] };
  if (restartedRuns.runs.length < firstRuns.runs.length + 1) {
    throw new Error('The restarted container did not retain and extend durable state.');
  }
  process.stdout.write('WOML Docker release candidate smoke passed.\n');
} finally {
  docker(['stop', '--time', '5', container], true);
  docker(['rm', '--force', container], true);
  docker(['volume', 'rm', '--force', volume], true);
  docker(['image', 'rm', '--force', image], true);
}
