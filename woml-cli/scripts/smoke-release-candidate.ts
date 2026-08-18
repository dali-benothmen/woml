import { mkdtemp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

import {
  nativePackageBinaryName,
  nativePackageName,
  type WomlNativeTarget,
  womlNativeTargets,
} from '../src/native-platform';

interface Arguments {
  readonly mainRoot: string;
  readonly nativeRoot: string;
  readonly target: WomlNativeTarget;
}

function parseArguments(values: readonly string[]): Arguments {
  const options = new Map<string, string>();
  for (let index = 0; index < values.length; index += 2) {
    const option = values[index];
    const value = values[index + 1];
    if (option === undefined || value === undefined || !option.startsWith('--')) {
      throw new Error(
        'Usage: smoke-release-candidate.ts --main-root <path> --native-root <path> --target <target>',
      );
    }
    options.set(option, value);
  }
  const mainRoot = options.get('--main-root');
  const nativeRoot = options.get('--native-root');
  const target = options.get('--target');
  if (
    mainRoot === undefined ||
    nativeRoot === undefined ||
    target === undefined ||
    !womlNativeTargets.includes(target as WomlNativeTarget)
  ) {
    throw new Error(
      'Usage: smoke-release-candidate.ts --main-root <path> --native-root <path> --target <target>',
    );
  }
  return {
    mainRoot: resolve(mainRoot),
    nativeRoot: resolve(nativeRoot),
    target: target as WomlNativeTarget,
  };
}

async function oneArchive(root: string): Promise<string> {
  const archives = (await readdir(root))
    .filter(name => name.endsWith('.tgz'))
    .map(name => join(root, name));
  if (archives.length !== 1) {
    throw new Error(`Expected one npm archive in ${root}, found ${archives.length}.`);
  }
  return archives[0]!;
}

async function filesBelow(root: string): Promise<string[]> {
  const files: string[] = [];
  for (const entry of await readdir(root, { withFileTypes: true })) {
    const path = join(root, entry.name);
    if (entry.isDirectory()) files.push(...(await filesBelow(path)));
    else files.push(path);
  }
  return files;
}

function run(command: readonly string[], cwd: string): string {
  const result = Bun.spawnSync(command, {
    cwd,
    stdout: 'pipe',
    stderr: 'pipe',
  });
  if (result.exitCode !== 0) {
    throw new Error(
      `Command failed (${command.join(' ')}):\n${result.stdout.toString()}${result.stderr.toString()}`,
    );
  }
  return result.stdout.toString();
}

async function main(): Promise<void> {
  const args = parseArguments(process.argv.slice(2));
  const mainArchive = await oneArchive(args.mainRoot);
  const nativeArchive = await oneArchive(args.nativeRoot);
  const stagedMainFiles = await filesBelow(args.mainRoot);
  if (stagedMainFiles.some(path => path.endsWith('.node'))) {
    throw new Error('The portable woml package contains a native binary.');
  }

  const directory = await mkdtemp(join(tmpdir(), `woml-candidate-${args.target}-`));
  const cache = join(directory, 'cache');
  const consumer = join(directory, 'consumer');
  try {
    await Promise.all([mkdir(cache), mkdir(consumer)]);
    await writeFile(
      join(consumer, 'package.json'),
      JSON.stringify({ name: 'woml-release-candidate-smoke', private: true }),
    );
    await writeFile(
      join(consumer, 'hello.woml'),
      `<woml>
  <workflow id="release-candidate-hello" name="Release candidate hello" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps><step id="hello"><script>return { message: "Hello World" };</script></step></steps>
  </workflow>
</woml>\n`,
    );

    const installEnvironment = {
      ...process.env,
      BUN_INSTALL_CACHE_DIR: cache,
    };
    const install = (): void => {
      const result = Bun.spawnSync(
        [Bun.which('bun')!, 'add', mainArchive, nativeArchive, '--no-save'],
        { cwd: consumer, env: installEnvironment, stdout: 'pipe', stderr: 'pipe' },
      );
      if (result.exitCode !== 0) {
        throw new Error(
          `Clean candidate installation failed:\n${result.stdout.toString()}${result.stderr.toString()}`,
        );
      }
    };
    install();

    const packageRoot = join(consumer, 'node_modules', '@woml-org', 'woml');
    const { version: expectedVersion } = JSON.parse(
      await readFile(join(packageRoot, 'package.json'), 'utf8'),
    ) as { readonly version: string };
    const nativeRoot = join(
      consumer,
      'node_modules',
      ...nativePackageName(args.target).split('/'),
    );
    if ((await filesBelow(packageRoot)).some(path => path.endsWith('.node'))) {
      throw new Error('The installed portable woml package contains a native binary.');
    }
    const nativeBinary = join(nativeRoot, nativePackageBinaryName(args.target));
    if (!(await Bun.file(nativeBinary).exists())) {
      throw new Error(`The installed native package is missing ${nativeBinary}.`);
    }

    const cli = join(packageRoot, 'dist', 'cli.js');
    const command = (...values: string[]): string =>
      run([Bun.which('bun')!, cli, ...values], consumer);
    if (command('--version').trim() !== `woml ${expectedVersion}`) {
      throw new Error(`The installed CLI did not report version ${expectedVersion}.`);
    }
    if (!command('--help').includes('woml run')) {
      throw new Error('The installed CLI help is incomplete.');
    }
    if (!command('check', 'hello.woml').includes('WOML check passed')) {
      throw new Error('The installed compiler did not validate hello.woml.');
    }
    const result = command(
      'test',
      'hello.woml',
      '--state',
      join(directory, 'state.sqlite'),
    );
    if (result !== '{"message":"Hello World"}\n') {
      throw new Error(`The installed runtime returned an unexpected result: ${result}`);
    }

    install();
    if (command('--version').trim() !== `woml ${expectedVersion}`) {
      throw new Error(`The reinstalled CLI did not report version ${expectedVersion}.`);
    }
    process.stdout.write(`WOML ${args.target} release candidate smoke passed.\n`);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}

await main();
