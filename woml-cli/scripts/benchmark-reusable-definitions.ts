import { cp, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';

const projectRoot = resolve(import.meta.dir, '../..');
const executable = resolve(projectRoot, 'woml-cli/dist/cli.js');
const fixtures = resolve(projectRoot, 'woml/tests/fixtures');
const temporaryRoot = await mkdtemp(
  resolve(tmpdir(), 'woml-reusable-benchmark-')
);

function invoke(args: readonly string[], cwd = projectRoot): number {
  const started = performance.now();
  const result = Bun.spawnSync([process.execPath, executable, ...args], {
    cwd,
    env: {
      ...process.env,
      WOML_SECRETS_PROVIDER: 'env',
      WOML_SECRET_REUSABLE_TEST_TOKEN: 'benchmark-secret',
    },
    stdout: 'pipe',
    stderr: 'pipe',
  });
  const duration = performance.now() - started;
  if (result.exitCode !== 0) {
    throw new Error(
      `Benchmark command failed: woml ${args.join(' ')}\n${result.stderr.toString()}`
    );
  }
  return duration;
}

try {
  const project = resolve(temporaryRoot, 'project');
  await cp(resolve(fixtures, 'reusable-production'), project, {
    recursive: true,
  });
  const composition = resolve(project, 'composition.woml');
  const provider = resolve(
    fixtures,
    'reusable-definitions/custom-provider-workflow.woml'
  );
  const exactSwitch = resolve(
    fixtures,
    'reusable-definitions/switch-control.reviewed.woml'
  );

  const coldCompilationMs = invoke(['check', composition, '--json'], project);
  const warmSamples = Array.from({ length: 5 }, () =>
    invoke(['check', composition, '--json'], project)
  );
  const warmCompilationMs =
    warmSamples.reduce((total, value) => total + value, 0) /
    warmSamples.length;
  const switchCompilationMs = invoke(['check', exactSwitch, '--json']);
  const providerCompilationMs = invoke(['check', provider, '--json']);
  const customStepExecutionMs = invoke(
    [
      'test',
      composition,
      '--state',
      resolve(temporaryRoot, 'execution.sqlite'),
    ],
    project
  );

  const manyRoot = resolve(temporaryRoot, 'many');
  await cp(project, manyRoot, { recursive: true });
  const imports: string[] = [];
  const usages: string[] = [];
  for (let index = 0; index < 24; index += 1) {
    const alias = `component-${index}`;
    await writeFile(
      resolve(manyRoot, `${alias}.woml`),
      `<woml><step><script>return { index: ${index} };</script></step></woml>`
    );
    imports.push(`<module name="${alias}" from="./${alias}.woml" />`);
    usages.push(`<${alias} id="component${index}" />`);
  }
  const manyWorkflow = resolve(manyRoot, 'many.woml');
  await writeFile(
    manyWorkflow,
    `<woml><imports>${imports.join('')}</imports><workflow id="many-components"><steps>${usages.join('')}<step id="done"><script>return true;</script></step></steps></workflow></woml>`
  );
  const manyDefinitionCompilationMs = invoke(
    ['check', manyWorkflow, '--json'],
    manyRoot
  );

  const measurements = {
    profile: 'woml.reusable-definitions-benchmark/v1',
    coldCompilationMs,
    warmCompilationMs,
    switchCompilationMs,
    providerCompilationMs,
    customStepExecutionMs,
    manyDefinitionCompilationMs,
    manyDefinitionCount: 24,
  };
  const budgets = {
    coldCompilationMs: 3_000,
    warmCompilationMs: 1_500,
    switchCompilationMs: 1_500,
    providerCompilationMs: 2_000,
    customStepExecutionMs: 5_000,
    manyDefinitionCompilationMs: 4_000,
  };
  for (const [name, maximum] of Object.entries(budgets)) {
    const observed = measurements[name as keyof typeof budgets] as number;
    if (observed > maximum) {
      throw new Error(
        `${name} exceeded its publication budget: ${observed.toFixed(2)}ms > ${maximum}ms`
      );
    }
  }
  console.log(JSON.stringify({ ...measurements, budgets }, null, 2));
} finally {
  await rm(temporaryRoot, { recursive: true, force: true });
}
