import { readFileSync, realpathSync, statSync } from 'node:fs';
import { dirname, extname, isAbsolute, relative, resolve, sep } from 'node:path';

import { parse } from 'acorn';

import {
  inspectValidatedWomlDocument,
  type ValidatedModuleDeclaration,
} from './compiler';
import {
  SourceFile,
  WomlCompileError,
  type SourceSpan,
  type WomlDiagnostic,
  type WomlSourceDocument,
} from './source';

export const WOML_DEFINITION_PACKAGE_PROFILE =
  'woml.definition-package/v1' as const;
export const WOML_MODULE_RESOLVER_PROFILE =
  'woml.module-resolver/v1' as const;

export interface WomlModuleResolverOptions {
  /** Absolute or working-directory-relative path of the importing WOML file. */
  readonly sourcePath?: string;
  /** Files outside this directory, including symlink targets, are rejected. */
  readonly projectRoot?: string;
}

export interface WomlDefinitionPackageModuleV1 {
  readonly name: string;
  readonly entrypoint: string;
  readonly exports: readonly string[];
}

export interface WomlDefinitionPackageSourceV1 {
  readonly path: string;
  readonly mediaType:
    | 'application/woml+xml'
    | 'text/javascript'
    | 'text/typescript';
  readonly digest: string;
  readonly dependencies: readonly string[];
}

export interface WomlDefinitionPackageV1 {
  readonly schemaVersion: 1;
  readonly profile: typeof WOML_DEFINITION_PACKAGE_PROFILE;
  readonly executable: false;
  readonly workflow: {
    readonly id: string;
    readonly source: string;
  };
  readonly modules: readonly WomlDefinitionPackageModuleV1[];
  readonly sources: readonly WomlDefinitionPackageSourceV1[];
  readonly compiler: {
    readonly name: 'woml';
    readonly version: '0.1.0';
    readonly resolverProfile: typeof WOML_MODULE_RESOLVER_PROFILE;
  };
  readonly permissions: {
    readonly secrets: readonly string[];
    readonly networkOrigins: readonly string[];
  };
  readonly rootHash: string;
}

interface AstNode {
  readonly type: string;
  readonly start: number;
  readonly end: number;
  readonly name?: string;
  readonly [key: string]: unknown;
}

interface ResolvedSource {
  readonly absolutePath: string;
  readonly portablePath: string;
  readonly source: string;
  readonly mediaType: 'text/javascript' | 'text/typescript';
  readonly digest: string;
  readonly dependencies: readonly string[];
  readonly functionExports: readonly string[];
  readonly runtimeExports: readonly string[];
}

const MAX_MODULE_SOURCES = 512;
const MAX_MODULE_SOURCE_BYTES = 16 * 1024 * 1024;

function isAstNode(value: unknown): value is AstNode {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as { type?: unknown }).type === 'string' &&
    typeof (value as { start?: unknown }).start === 'number' &&
    typeof (value as { end?: unknown }).end === 'number'
  );
}

function astChildren(node: AstNode): readonly AstNode[] {
  const result: AstNode[] = [];
  for (const [key, value] of Object.entries(node)) {
    if (key === 'type' || key === 'start' || key === 'end') continue;
    if (isAstNode(value)) result.push(value);
    else if (Array.isArray(value)) {
      for (const item of value) if (isAstNode(item)) result.push(item);
    }
  }
  return result;
}

function commonJsNode(root: AstNode): AstNode | undefined {
  const pending = [root];
  while (pending.length > 0) {
    const node = pending.pop()!;
    if (node.type === 'CallExpression') {
      const callee = node.callee;
      if (isAstNode(callee) && callee.type === 'Identifier' && callee.name === 'require') {
        return node;
      }
    }
    if (node.type === 'MemberExpression') {
      const object = node.object;
      const property = node.property;
      const directExports =
        isAstNode(object) && object.type === 'Identifier' && object.name === 'exports';
      const moduleExports =
        isAstNode(object) &&
        object.type === 'Identifier' &&
        object.name === 'module' &&
        isAstNode(property) &&
        ((property.type === 'Identifier' && property.name === 'exports') ||
          (property.type === 'Literal' && property.value === 'exports'));
      if (directExports || moduleExports) return node;
    }
    pending.push(...astChildren(node));
  }
  return undefined;
}

function canonicalJson(value: unknown): string {
  if (value === null || typeof value === 'boolean' || typeof value === 'string') {
    return JSON.stringify(value);
  }
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) {
      throw new TypeError('Canonical WOML package values must be finite JSON numbers.');
    }
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) {
    return `[${value.map(canonicalJson).join(',')}]`;
  }
  if (typeof value === 'object' && value !== null) {
    const record = value as Record<string, unknown>;
    return `{${Object.keys(record)
      .sort()
      .map(key => `${JSON.stringify(key)}:${canonicalJson(record[key])}`)
      .join(',')}}`;
  }
  throw new TypeError('Canonical WOML package values must be JSON-compatible.');
}

function sha256(value: string): string {
  return `sha256:${new Bun.CryptoHasher('sha256').update(value).digest('hex')}`;
}

function portablePath(projectRoot: string, absolutePath: string): string {
  const path = relative(projectRoot, absolutePath).split(sep).join('/');
  return path.length === 0 ? '.' : path;
}

function isWithin(root: string, candidate: string): boolean {
  const path = relative(root, candidate);
  return path === '' || (!path.startsWith(`..${sep}`) && path !== '..' && !isAbsolute(path));
}

function compileDiagnostic(
  file: string,
  code: string,
  message: string,
  location: SourceSpan,
  hint?: string
): WomlCompileError {
  const diagnostic: WomlDiagnostic = {
    code,
    phase: 'compile',
    message,
    file,
    location,
    ...(hint === undefined ? {} : { hint }),
  };
  return new WomlCompileError(diagnostic);
}

function failAtDeclaration(
  document: WomlSourceDocument,
  declaration: ValidatedModuleDeclaration,
  code: string,
  message: string,
  hint?: string
): never {
  throw compileDiagnostic(
    document.file,
    code,
    message,
    declaration.fromAttribute.valueSpan,
    hint
  );
}

function sourceSpan(sourceFile: SourceFile, source: string, needle?: string): SourceSpan {
  const offset = needle === undefined ? 0 : Math.max(0, source.indexOf(needle));
  return sourceFile.span(offset, Math.min(source.length, offset + Math.max(1, needle?.length ?? 1)));
}

function safeRealPath(
  requestedPath: string,
  projectRoot: string,
  document: WomlSourceDocument,
  declaration: ValidatedModuleDeclaration
): string {
  if (!isWithin(projectRoot, requestedPath)) {
    failAtDeclaration(
      document,
      declaration,
      'WOML_MODULE_PATH_ESCAPE',
      `Module source "${declaration.from}" escapes the project boundary.`,
      `Project boundary: ${projectRoot}`
    );
  }
  let realPath: string;
  try {
    const stat = statSync(requestedPath);
    if (!stat.isFile()) throw new Error('not a file');
    realPath = realpathSync(requestedPath);
  } catch {
    failAtDeclaration(
      document,
      declaration,
      'WOML_MODULE_FILE_NOT_FOUND',
      `Module source "${declaration.from}" does not resolve to a readable file.`
    );
  }
  if (!isWithin(projectRoot, realPath)) {
    failAtDeclaration(
      document,
      declaration,
      'WOML_MODULE_SYMLINK_ESCAPE',
      `Module source "${declaration.from}" resolves outside the project boundary.`,
      'Move the source into the project instead of following an external symlink.'
    );
  }
  return realPath;
}

function parseModuleSource(
  absolutePath: string,
  projectRoot: string,
  source: string
): {
  readonly imports: readonly { readonly kind: string; readonly path: string }[];
  readonly runtimeExports: readonly string[];
  readonly functionExports: readonly string[];
} {
  const portable = portablePath(projectRoot, absolutePath);
  const file = new SourceFile(portable, source);
  const loader = extname(absolutePath) === '.ts' ? 'ts' : 'js';
  const transpiler = new Bun.Transpiler({ loader });
  let scan: ReturnType<Bun.Transpiler['scan']>;
  let javascript: string;
  try {
    scan = transpiler.scan(source);
    javascript = transpiler.transformSync(source);
  } catch (error) {
    throw compileDiagnostic(
      portable,
      'WOML_MODULE_SYNTAX_INVALID',
      `Cannot parse module source: ${error instanceof Error ? error.message : String(error)}`,
      sourceSpan(file, source)
    );
  }

  let program: AstNode;
  try {
    const parsed = parse(javascript, {
      ecmaVersion: 'latest',
      sourceType: 'module',
      allowHashBang: true,
    });
    if (!isAstNode(parsed)) throw new Error('module parser returned no program');
    program = parsed;
  } catch (error) {
    throw compileDiagnostic(
      portable,
      'WOML_MODULE_SYNTAX_INVALID',
      `Cannot parse transpiled module: ${error instanceof Error ? error.message : String(error)}`,
      sourceSpan(file, source)
    );
  }

  const functionExports: string[] = [];
  const body = (program.body as readonly unknown[] | undefined) ?? [];
  for (const statement of body) {
    if (!isAstNode(statement)) continue;
    if (statement.type === 'ExportDefaultDeclaration') {
      throw compileDiagnostic(
        portable,
        'WOML_MODULE_DEFAULT_EXPORT_UNSUPPORTED',
        'WOML modules expose named function exports only; default exports are unsupported.',
        sourceSpan(file, source, 'export default')
      );
    }
    if (statement.type !== 'ExportNamedDeclaration') continue;
    const declaration = statement.declaration;
    if (isAstNode(declaration) && declaration.type === 'FunctionDeclaration') {
      if (declaration.generator === true) {
        throw compileDiagnostic(
          portable,
          'WOML_MODULE_GENERATOR_EXPORT_UNSUPPORTED',
          'Generator functions are not part of the Module v1 public surface.',
          sourceSpan(file, source, 'export')
        );
      }
      const identifier = declaration.id;
      if (isAstNode(identifier) && identifier.type === 'Identifier' && identifier.name !== undefined) {
        functionExports.push(identifier.name);
      }
      continue;
    }
    throw compileDiagnostic(
      portable,
      'WOML_MODULE_EXPORT_UNSUPPORTED',
      'Export runtime functions directly with `export function name(...)` or `export async function name(...)`.',
      sourceSpan(file, source, 'export'),
      'Default exports, export lists, re-exports, classes, constants, and CommonJS exports are outside Module profile v1.'
    );
  }

  if (commonJsNode(program) !== undefined) {
    throw compileDiagnostic(
      portable,
      'WOML_MODULE_COMMONJS_UNSUPPORTED',
      'CommonJS require/exports are unsupported; use static ESM imports and direct named function exports.',
      sourceSpan(
        file,
        source,
        source.includes('module.exports')
          ? 'module.exports'
          : source.includes('require')
            ? 'require'
            : 'exports'
      )
    );
  }

  for (const imported of scan.imports) {
    if (imported.kind === 'dynamic-import') {
      throw compileDiagnostic(
        portable,
        'WOML_MODULE_DYNAMIC_IMPORT_UNSUPPORTED',
        `Dynamic import of "${imported.path}" is unsupported.`,
        sourceSpan(file, source, imported.path),
        'Use a static relative import statement so the dependency can be hashed before activation.'
      );
    }
    if (imported.kind !== 'import-statement') {
      throw compileDiagnostic(
        portable,
        'WOML_MODULE_IMPORT_UNSUPPORTED',
        `Import form "${imported.kind}" is unsupported in Module profile v1.`,
        sourceSpan(file, source, imported.path)
      );
    }
  }

  return {
    imports: scan.imports,
    runtimeExports: [...scan.exports].sort(),
    functionExports: functionExports.sort(),
  };
}

export function buildWomlDefinitionPackage(
  document: WomlSourceDocument,
  options: WomlModuleResolverOptions = {}
): WomlDefinitionPackageV1 {
  const validated = inspectValidatedWomlDocument(document);
  const sourcePath = resolve(options.sourcePath ?? document.file);
  const lexicalProjectRoot = resolve(options.projectRoot ?? dirname(sourcePath));
  let projectRoot: string;
  try {
    projectRoot = realpathSync(lexicalProjectRoot);
  } catch {
    throw compileDiagnostic(
      document.file,
      'WOML_MODULE_PROJECT_ROOT_INVALID',
      `Module project root "${lexicalProjectRoot}" is not a readable directory.`,
      document.root.openTagSpan
    );
  }
  if (!isWithin(projectRoot, sourcePath)) {
    throw compileDiagnostic(
      document.file,
      'WOML_MODULE_DOCUMENT_ESCAPE',
      'The importing WOML document is outside the configured project boundary.',
      document.root.openTagSpan
    );
  }
  try {
    const realSourcePath = realpathSync(sourcePath);
    if (!statSync(realSourcePath).isFile() || !isWithin(projectRoot, realSourcePath)) {
      throw new Error('outside project boundary');
    }
  } catch {
    throw compileDiagnostic(
      document.file,
      'WOML_MODULE_DOCUMENT_INVALID',
      'The importing WOML document must be a readable file inside the project boundary.',
      document.root.openTagSpan
    );
  }

  const sources = new Map<string, ResolvedSource>();
  const visiting: string[] = [];
  const casePaths = new Map<string, string>();
  let totalSourceBytes = 0;

  const resolveSource = (
    absolutePath: string,
    declaration: ValidatedModuleDeclaration
  ): ResolvedSource => {
    const realPath = safeRealPath(absolutePath, projectRoot, document, declaration);
    const existing = sources.get(realPath);
    if (existing !== undefined) return existing;
    const cycleStart = visiting.indexOf(realPath);
    if (cycleStart !== -1) {
      const cycle = [...visiting.slice(cycleStart), realPath]
        .map(path => portablePath(projectRoot, path))
        .join(' -> ');
      failAtDeclaration(
        document,
        declaration,
        'WOML_MODULE_GRAPH_CYCLE',
        `Module dependency graph contains a cycle: ${cycle}.`
      );
    }

    const portable = portablePath(projectRoot, realPath);
    const caseKey = portable.toLocaleLowerCase('en-US');
    const caseExisting = casePaths.get(caseKey);
    if (caseExisting !== undefined && caseExisting !== portable) {
      failAtDeclaration(
        document,
        declaration,
        'WOML_MODULE_PATH_CASE_COLLISION',
        `Module paths "${caseExisting}" and "${portable}" differ by case only.`
      );
    }
    casePaths.set(caseKey, portable);

    const extension = extname(realPath);
    if (extension !== '.js' && extension !== '.ts') {
      failAtDeclaration(
        document,
        declaration,
        'WOML_MODULE_EXTENSION_UNSUPPORTED',
        `Module dependency "${portable}" must end in .js or .ts.`
      );
    }
    let source: string;
    try {
      source = readFileSync(realPath, 'utf8');
    } catch {
      failAtDeclaration(
        document,
        declaration,
        'WOML_MODULE_FILE_NOT_READABLE',
        `Module source "${portable}" could not be read.`
      );
    }
    if (sources.size + visiting.length >= MAX_MODULE_SOURCES) {
      failAtDeclaration(
        document,
        declaration,
        'WOML_MODULE_GRAPH_LIMIT_EXCEEDED',
        `Module graph exceeds the ${MAX_MODULE_SOURCES}-source limit.`
      );
    }
    totalSourceBytes += Buffer.byteLength(source);
    if (totalSourceBytes > MAX_MODULE_SOURCE_BYTES) {
      failAtDeclaration(
        document,
        declaration,
        'WOML_MODULE_SIZE_LIMIT_EXCEEDED',
        'Module graph exceeds the 16 MiB source limit.'
      );
    }
    const analysis = parseModuleSource(realPath, projectRoot, source);
    visiting.push(realPath);
    const dependencyPaths: string[] = [];
    for (const imported of analysis.imports) {
      if (!imported.path.startsWith('./') && !imported.path.startsWith('../')) {
        const sourceFile = new SourceFile(portable, source);
        throw compileDiagnostic(
          portable,
          'WOML_MODULE_PACKAGE_IMPORT_UNAVAILABLE',
          `Package import "${imported.path}" is unavailable until MS5.`,
          sourceSpan(sourceFile, source, imported.path),
          'Use an explicit relative .js or .ts dependency in the local-module slice.'
        );
      }
      if (!/\.(?:js|ts)$/.test(imported.path)) {
        const sourceFile = new SourceFile(portable, source);
        throw compileDiagnostic(
          portable,
          'WOML_MODULE_IMPORT_EXTENSION_REQUIRED',
          `Static import "${imported.path}" requires an explicit .js or .ts extension.`,
          sourceSpan(sourceFile, source, imported.path)
        );
      }
      const dependency = resolveSource(resolve(dirname(realPath), imported.path), declaration);
      dependencyPaths.push(dependency.portablePath);
    }
    visiting.pop();

    const resolvedSource: ResolvedSource = {
      absolutePath: realPath,
      portablePath: portable,
      source,
      mediaType: extension === '.ts' ? 'text/typescript' : 'text/javascript',
      digest: sha256(source),
      dependencies: [...new Set(dependencyPaths)].sort(),
      functionExports: analysis.functionExports,
      runtimeExports: analysis.runtimeExports,
    };
    sources.set(realPath, resolvedSource);
    return resolvedSource;
  };

  const modules: WomlDefinitionPackageModuleV1[] = [];
  const entrypointNames = new Map<string, string>();
  for (const declaration of validated.modules) {
    const entrypoint = resolveSource(
      resolve(dirname(sourcePath), declaration.from),
      declaration
    );
    const previousName = entrypointNames.get(entrypoint.absolutePath);
    if (previousName !== undefined) {
      failAtDeclaration(
        document,
        declaration,
        'WOML_MODULE_SOURCE_DUPLICATE',
        `Module source "${declaration.from}" is already exposed as "${previousName}".`,
        'Declare one stable services alias for each module entry point.'
      );
    }
    entrypointNames.set(entrypoint.absolutePath, declaration.name);
    const unsupported = entrypoint.runtimeExports.filter(
      name => !entrypoint.functionExports.includes(name)
    );
    if (unsupported.length > 0) {
      const file = new SourceFile(entrypoint.portablePath, entrypoint.source);
      throw compileDiagnostic(
        entrypoint.portablePath,
        'WOML_MODULE_EXPORT_UNSUPPORTED',
        `Module runtime export "${unsupported[0]}" is not a direct named function export.`,
        sourceSpan(file, entrypoint.source, unsupported[0])
      );
    }
    if (entrypoint.functionExports.length === 0) {
      const file = new SourceFile(entrypoint.portablePath, entrypoint.source);
      throw compileDiagnostic(
        entrypoint.portablePath,
        'WOML_MODULE_EXPORTS_EMPTY',
        'A declared WOML module requires at least one named function export.',
        sourceSpan(file, entrypoint.source)
      );
    }
    modules.push({
      name: declaration.name,
      entrypoint: entrypoint.portablePath,
      exports: entrypoint.functionExports,
    });
  }

  const workflowPortablePath = portablePath(projectRoot, sourcePath);
  const packageSources: WomlDefinitionPackageSourceV1[] = [
    {
      path: workflowPortablePath,
      mediaType: 'application/woml+xml',
      digest: sha256(document.source),
      dependencies: modules.map(module => module.entrypoint).sort(),
    },
    ...[...sources.values()]
      .sort((left, right) => left.portablePath.localeCompare(right.portablePath))
      .map(source => ({
        path: source.portablePath,
        mediaType: source.mediaType,
        digest: source.digest,
        dependencies: source.dependencies,
      })),
  ];
  const unsigned = {
    schemaVersion: 1 as const,
    profile: WOML_DEFINITION_PACKAGE_PROFILE,
    executable: false as const,
    workflow: { id: validated.workflowId, source: workflowPortablePath },
    modules: modules.sort((left, right) => left.name.localeCompare(right.name)),
    sources: packageSources,
    compiler: {
      name: 'woml' as const,
      version: '0.1.0' as const,
      resolverProfile: WOML_MODULE_RESOLVER_PROFILE,
    },
    permissions: {
      secrets: [] as readonly string[],
      networkOrigins: [] as readonly string[],
    },
  };
  return { ...unsigned, rootHash: sha256(canonicalJson(unsigned)) };
}

export function canonicalizeWomlDefinitionPackage(
  definitionPackage: WomlDefinitionPackageV1
): string {
  return canonicalJson(definitionPackage);
}
