import {
  mkdtemp,
  mkdir,
  readFile,
  readdir,
  rm,
  stat,
  writeFile,
} from 'node:fs/promises';
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

// Checked by verify-process-release.ts. Keep this smoke in every native matrix job.
const PROCESS_CONTAINMENT_SMOKE_V1 = true;

function parseArguments(values: readonly string[]): Arguments {
  const options = new Map<string, string>();
  for (let index = 0; index < values.length; index += 2) {
    const option = values[index];
    const value = values[index + 1];
    if (
      option === undefined ||
      value === undefined ||
      !option.startsWith('--')
    ) {
      throw new Error(
        'Usage: smoke-release-candidate.ts --main-root <path> --native-root <path> --target <target>'
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
      'Usage: smoke-release-candidate.ts --main-root <path> --native-root <path> --target <target>'
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
    throw new Error(
      `Expected one npm archive in ${root}, found ${archives.length}.`
    );
  }
  return archives[0]!;
}

async function filesBelow(root: string): Promise<string[]> {
  const files: string[] = [];
  for (const entry of await readdir(root, { withFileTypes: true })) {
    const path = join(root, entry.name);
    if (entry.isDirectory()) {
      if (entry.name !== 'node_modules') {
        files.push(...(await filesBelow(path)));
      }
    } else files.push(path);
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
      `Command failed (${command.join(' ')}):\n${result.stdout.toString()}${result.stderr.toString()}`
    );
  }
  return result.stdout.toString();
}

function runRaw(
  command: readonly string[],
  cwd: string
): ReturnType<typeof Bun.spawnSync> {
  return Bun.spawnSync(command, { cwd, stdout: 'pipe', stderr: 'pipe' });
}

async function main(): Promise<void> {
  const args = parseArguments(process.argv.slice(2));
  const mainArchive = await oneArchive(args.mainRoot);
  const nativeArchive = await oneArchive(args.nativeRoot);
  const stagedMainFiles = await filesBelow(args.mainRoot);
  if (stagedMainFiles.some(path => path.endsWith('.node'))) {
    throw new Error('The portable woml package contains a native binary.');
  }

  const directory = await mkdtemp(
    join(tmpdir(), `woml-candidate-${args.target}-`)
  );
  const cache = join(directory, 'cache');
  const consumer = join(directory, 'consumer');
  try {
    await Promise.all([mkdir(cache), mkdir(consumer)]);
    await writeFile(
      join(consumer, 'package.json'),
      JSON.stringify({ name: 'woml-release-candidate-smoke', private: true })
    );
    await writeFile(
      join(consumer, 'hello.woml'),
      `<woml>
  <workflow id="release-candidate-hello" name="Release candidate hello" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps><step id="hello"><script>return { message: "Hello World" };</script></step></steps>
  </workflow>
</woml>\n`
    );
    await writeFile(
      join(consumer, 'for-each.woml'),
      `<woml>
  <workflow id="release-candidate-for-each" name="Release candidate for each" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps>
      <step id="load"><script>return { names: ["Grace", "Ada", "Linus"] };</script></step>
      <for-each id="greetings" items="{{context.steps.load.names}}" concurrency="2">
        <step id="greet"><script>return { index: context.iteration.index, message: \`Hello \${context.item}\` };</script></step>
        <result value="{{context.steps.greet}}" />
      </for-each>
      <step id="summary"><script>return context.steps.greetings;</script></step>
    </steps>
  </workflow>
</woml>\n`
    );
    await writeFile(
      join(consumer, 'durable-wait.woml'),
      `<woml>
  <workflow id="release-candidate-wait" name="Release candidate durable wait" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps>
      <wait id="pause" duration="1s" />
      <step id="done"><script>return { status: context.steps.pause.status };</script></step>
    </steps>
  </workflow>
</woml>\n`
    );
    // Node is installed in every release matrix job. A relative script path
    // exercises the approved Process working directory and literal arguments.
    await writeFile(
      join(consumer, 'process-literal.woml'),
      `<woml>
  <workflow id="release-candidate-process" name="Release candidate process" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps><step id="literal"><script>
      const result = await services.process.run({
        executable: "node",
        args: ["./echo arguments 雪.js", "space value", "café 雪", "\\\"quoted\\\"", "; echo not-a-shell"],
        timeout: "10s",
        // This fixture has no secrets. Accept Node's normal error exit so a
        // Windows release failure reports its actual stderr and exit status.
        acceptExitCodes: [0, 1]
      }, { name: "release-literal-arguments" });
      return result.exitCode === 0
        ? { exitCode: result.exitCode, arguments: JSON.parse(result.stdout) }
        : { exitCode: result.exitCode, stderr: result.stderr.slice(0, 2048) };
    </script></step></steps>
  </workflow>
</woml>\n`
    );
    await writeFile(
      join(consumer, 'echo arguments 雪.js'),
      'process.stdout.write(JSON.stringify(process.argv.slice(2)) + "\\r\\n");\n'
    );
    await writeFile(
      join(consumer, 'process-tree.woml'),
      `<woml>
  <workflow id="release-candidate-process-tree" name="Release process containment" version="1.0.0">
    <triggers><manual id="start" /></triggers>
    <steps><step id="tree"><script>
      return await services.process.run({
        executable: "node",
        args: ["./tree-parent.js", "process-heartbeat"],
        timeout: "2s"
      }, { name: "release-process-tree" });
    </script></step></steps>
  </workflow>
</woml>\n`
    );
    await writeFile(
      join(consumer, 'tree-parent.js'),
      `const { spawn } = require("node:child_process");
const child = spawn(process.execPath, ["./tree-grandchild.js", process.argv[2]], {
  stdio: "ignore"
});
child.on("error", error => { process.stderr.write(String(error)); process.exitCode = 1; });
child.on("exit", code => { process.exitCode = code ?? 1; });\n`
    );
    await writeFile(
      join(consumer, 'tree-grandchild.js'),
      `const { appendFileSync } = require("node:fs");
const path = process.argv[2];
setInterval(() => appendFileSync(path, "x"), 20);\n`
    );

    const installEnvironment = {
      ...process.env,
      BUN_INSTALL_CACHE_DIR: cache,
    };
    const install = (): void => {
      const result = Bun.spawnSync(
        [Bun.which('bun')!, 'add', mainArchive, nativeArchive, '--no-save'],
        {
          cwd: consumer,
          env: installEnvironment,
          stdout: 'pipe',
          stderr: 'pipe',
        }
      );
      if (result.exitCode !== 0) {
        throw new Error(
          `Clean candidate installation failed:\n${result.stdout.toString()}${result.stderr.toString()}`
        );
      }
    };
    install();

    const packageRoot = join(consumer, 'node_modules', 'woml-cli');
    const { version: expectedVersion } = JSON.parse(
      await readFile(join(packageRoot, 'package.json'), 'utf8')
    ) as { readonly version: string };
    const nativeRoot = join(
      consumer,
      'node_modules',
      ...nativePackageName(args.target).split('/')
    );
    if ((await filesBelow(packageRoot)).some(path => path.endsWith('.node'))) {
      throw new Error(
        'The installed portable woml package contains a native binary.'
      );
    }
    const nativeBinary = join(nativeRoot, nativePackageBinaryName(args.target));
    if (!(await Bun.file(nativeBinary).exists())) {
      throw new Error(
        `The installed native package is missing ${nativeBinary}.`
      );
    }

    const cli = join(packageRoot, 'dist', 'cli.js');
    const command = (...values: string[]): string =>
      run([Bun.which('bun')!, cli, ...values], consumer);
    if (command('--version').trim() !== `woml ${expectedVersion}`) {
      throw new Error(
        `The installed CLI did not report version ${expectedVersion}.`
      );
    }
    if (!command('--help').includes('woml run')) {
      throw new Error('The installed CLI help is incomplete.');
    }
    if (!command('check', 'hello.woml').includes('WOML check passed')) {
      throw new Error('The installed compiler did not validate hello.woml.');
    }
    if (!command('check', 'for-each.woml').includes('WOML check passed')) {
      throw new Error('The installed compiler did not validate for-each.woml.');
    }
    if (!command('check', 'durable-wait.woml').includes('WOML check passed')) {
      throw new Error(
        'The installed compiler did not validate durable-wait.woml.'
      );
    }
    if (
      !command('check', 'process-literal.woml').includes('WOML check passed')
    ) {
      throw new Error(
        'The installed compiler did not validate process-literal.woml.'
      );
    }
    const result = command(
      'test',
      'hello.woml',
      '--state',
      join(directory, 'state.sqlite')
    );
    if (result !== '{"message":"Hello World"}\n') {
      throw new Error(
        `The installed runtime returned an unexpected result: ${result}`
      );
    }
    const waitResult = command(
      'test',
      'durable-wait.woml',
      '--state',
      join(directory, 'durable-wait-state.sqlite')
    );
    if (waitResult !== '{"status":"elapsed"}\n') {
      throw new Error(
        `The installed runtime returned an unexpected durable-wait result: ${waitResult}`
      );
    }
    const forEachResult = command(
      'test',
      'for-each.woml',
      '--state',
      join(directory, 'for-each-state.sqlite')
    );
    const expectedForEach = {
      total: 3,
      succeeded: 3,
      results: [
        { index: 0, message: 'Hello Grace' },
        { index: 1, message: 'Hello Ada' },
        { index: 2, message: 'Hello Linus' },
      ],
    };
    if (
      JSON.stringify(JSON.parse(forEachResult)) !==
      JSON.stringify(expectedForEach)
    ) {
      throw new Error(
        `The installed runtime returned an unexpected for-each result: ${forEachResult}`
      );
    }
    let processResult: string;
    try {
      processResult = command(
        'test',
        'process-literal.woml',
        '--allow-process',
        'node',
        '--state',
        join(directory, 'process-state.sqlite')
      );
    } catch (error) {
      // A failed Process attempt intentionally hides child output. This
      // isolated, synthetic helper has no secrets, so probe it directly with
      // the same empty environment to distinguish child startup from policy,
      // containment, and working-directory failures in the release matrix.
      const node = Bun.which('node');
      const probe = node === null
        ? 'node is unavailable'
        : (() => {
            try {
              const result = Bun.spawnSync(
                [node, './echo arguments 雪.js', 'space value', 'café 雪'],
                { cwd: consumer, env: {}, stdout: 'pipe', stderr: 'pipe' }
              );
              return `exit=${result.exitCode}; stdout=${result.stdout.toString().slice(0, 2048)}; stderr=${result.stderr.toString().slice(0, 2048)}`;
            } catch (probeError) {
              return `failed to start: ${String(probeError)}`;
            }
          })();
      throw new Error(`${String(error)}\nDirect isolated Node probe: ${probe}`);
    }
    const expectedProcess = {
      exitCode: 0,
      arguments: ['space value', 'café 雪', '"quoted"', '; echo not-a-shell'],
    };
    if (
      JSON.stringify(JSON.parse(processResult)) !==
      JSON.stringify(expectedProcess)
    ) {
      throw new Error(
        `The installed Process v1 runtime changed literal arguments: ${processResult}`
      );
    }

    if (PROCESS_CONTAINMENT_SMOKE_V1) {
      const containment = runRaw(
        [
          Bun.which('bun')!,
          cli,
          'test',
          'process-tree.woml',
          '--allow-process',
          'node',
          '--state',
          join(directory, 'process-tree-state.sqlite'),
        ],
        consumer
      );
      const containmentOutput = `${containment.stdout.toString()}${containment.stderr.toString()}`;
      if (
        containment.exitCode === 0 ||
        !containmentOutput.includes('WOML_PROCESS_TIMED_OUT')
      ) {
        throw new Error(
          `The packaged Process v1 timeout smoke did not fail safely:\n${containmentOutput}`
        );
      }
      const heartbeat = join(consumer, 'process-heartbeat');
      let heartbeatCreated = false;
      for (let attempt = 0; attempt < 20; attempt += 1) {
        try {
          await stat(heartbeat);
          heartbeatCreated = true;
          break;
        } catch {
          await Bun.sleep(25);
        }
      }
      if (!heartbeatCreated) {
        throw new Error('The packaged Process v1 child tree never started.');
      }
      await Bun.sleep(200);
      const first = (await stat(heartbeat)).size;
      await Bun.sleep(200);
      const second = (await stat(heartbeat)).size;
      if (first !== second) {
        throw new Error(
          'A packaged Process v1 descendant survived timeout containment.'
        );
      }
    }

    install();
    if (command('--version').trim() !== `woml ${expectedVersion}`) {
      throw new Error(
        `The reinstalled CLI did not report version ${expectedVersion}.`
      );
    }
    process.stdout.write(
      `WOML ${args.target} release candidate smoke passed.\n`
    );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}

await main();
