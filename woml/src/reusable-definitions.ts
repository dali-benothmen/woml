import { createHash } from 'node:crypto';
import { readFileSync, realpathSync, statSync } from 'node:fs';
import { dirname, isAbsolute, relative, resolve, sep } from 'node:path';

import { parseWoml } from './parser';
import {
  SourceFile,
  WomlCompileError,
  WomlValidationError,
  type SourceSpan,
  type WomlDiagnostic,
  type WomlSourceAttribute,
  type WomlSourceDocument,
  type WomlSourceElement,
} from './source';

export type WomlDocumentKind = 'workflow' | 'reusable-step' | 'notification-provider';

export interface WomlImportDeclaration {
  readonly name: string;
  readonly from: string;
  readonly kind: 'script-module' | 'reusable-definition';
  readonly element: WomlSourceElement;
}

export interface WomlPropDeclaration {
  readonly name: string;
  readonly bindingName: string;
  readonly required: boolean;
  readonly secret: boolean;
  readonly element: WomlSourceElement;
}

export interface WomlDocumentInspection {
  readonly kind: WomlDocumentKind;
  readonly sourcePath: string;
  readonly imports: readonly WomlImportDeclaration[];
  readonly props: readonly WomlPropDeclaration[];
  readonly definition: WomlSourceElement;
  readonly lifecycle?: WomlSourceElement;
}

export interface WomlResolvedReusableDefinition {
  readonly alias: string;
  readonly sourcePath: string;
  readonly digest: string;
  readonly kind: 'reusable-step' | 'notification-provider';
  readonly props: readonly Omit<WomlPropDeclaration, 'element'>[];
}

export interface WomlReusableDefinitionGraph {
  readonly root: WomlDocumentInspection;
  readonly definitions: readonly WomlResolvedReusableDefinition[];
  readonly sources: readonly {
    readonly path: string;
    readonly digest: string;
    readonly kind: WomlDocumentKind | 'script-module';
  }[];
  readonly rootHash: string;
}

export interface WomlReusableDefinitionResolverOptions {
  readonly sourcePath?: string;
  readonly projectRoot?: string;
}

const lowerCamelPattern = /^[a-z][A-Za-z0-9]*$/;
const kebabPattern = /^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$/;
const relativeScriptPattern =
  /^(?:\.\/|\.\.\/)(?:[A-Za-z0-9._-]+\/)*[A-Za-z0-9._-]+\.(?:js|ts)$/;
const relativeWomlPattern =
  /^(?:\.\/|\.\.\/)(?:[A-Za-z0-9._-]+\/)*[A-Za-z0-9._-]+\.woml$/;
const exactSecretReference = /^\{\{secrets\.([A-Z][A-Z0-9_]*)\}\}$/;
const exactContextReference =
  /^\{\{context\.(?:payload(?:\.[A-Za-z_$][A-Za-z0-9_$]*)*|steps\.[a-z][A-Za-z0-9]*(?:\.[A-Za-z_$][A-Za-z0-9_$]*)*)\}\}$/;
const reservedBindingNames = new Set(['__proto__', 'prototype', 'constructor']);
const reservedCustomTags = new Set([
  'woml',
  'imports',
  'module',
  'props',
  'prop',
  'workflow',
  'config',
  'lifecycle',
  'on-start',
  'on-step-start',
  'on-step-success',
  'on-step-failure',
  'on-step-complete',
  'on-success',
  'on-error',
  'on-failure',
  'on-cancel',
  'on-complete',
  'triggers',
  'manual',
  'webhook',
  'slack',
  'schedule',
  'interval',
  'event',
  'schema',
  'steps',
  'step',
  'script',
  'choose',
  'switch',
  'case',
  'default',
  'when',
  'otherwise',
  'result',
  'parallel',
  'for-each',
  'fork',
  'branch',
  'approval',
  'notify',
  'provider',
  'when-approved',
  'when-rejected',
]);

function diagnostic(
  document: WomlSourceDocument,
  phase: 'validation' | 'compile',
  code: string,
  message: string,
  span: SourceSpan,
  hint?: string
): WomlDiagnostic {
  return {
    phase,
    code,
    message,
    file: document.file,
    location: span,
    ...(hint === undefined ? {} : { hint }),
  };
}

function failValidation(
  document: WomlSourceDocument,
  code: string,
  message: string,
  span: SourceSpan,
  hint?: string
): never {
  throw new WomlValidationError(
    diagnostic(document, 'validation', code, message, span, hint)
  );
}

function failCompile(
  document: WomlSourceDocument,
  code: string,
  message: string,
  span: SourceSpan,
  hint?: string
): never {
  throw new WomlCompileError(
    diagnostic(document, 'compile', code, message, span, hint)
  );
}

function elementChildren(
  document: WomlSourceDocument,
  element: WomlSourceElement
): readonly WomlSourceElement[] {
  const result: WomlSourceElement[] = [];
  for (const child of element.children) {
    if (child.kind !== 'element') {
      failValidation(
        document,
        'WOML_UNEXPECTED_CONTENT',
        `<${element.name}> may contain WOML elements only.`,
        child.span
      );
    }
    result.push(child);
  }
  return result;
}

function ensureNoAttributes(
  document: WomlSourceDocument,
  element: WomlSourceElement
): void {
  const attribute = Object.values(element.attributes)[0];
  if (attribute !== undefined) {
    failValidation(
      document,
      'WOML_UNKNOWN_ATTRIBUTE',
      `<${element.name}> does not accept attribute "${attribute.name}".`,
      attribute.nameSpan
    );
  }
}

function ensureOnlyAttributes(
  document: WomlSourceDocument,
  element: WomlSourceElement,
  allowed: ReadonlySet<string>
): void {
  const unknown = Object.values(element.attributes).find(
    attribute => !allowed.has(attribute.name)
  );
  if (unknown !== undefined) {
    failValidation(
      document,
      'WOML_UNKNOWN_ATTRIBUTE',
      `Unknown attribute "${unknown.name}" on <${element.name}>.`,
      unknown.nameSpan
    );
  }
}

function ensureEmpty(
  document: WomlSourceDocument,
  element: WomlSourceElement
): void {
  if (element.children.length > 0) {
    failValidation(
      document,
      'WOML_REUSABLE_TAG_CHILDREN_UNSUPPORTED',
      `<${element.name}> must be empty; reusable structural children are not supported in this release.`,
      element.children[0].span,
      'Pass declared props as attributes. General custom structural tags are postponed.'
    );
  }
}

function requiredAttribute(
  document: WomlSourceDocument,
  element: WomlSourceElement,
  name: string
): WomlSourceAttribute {
  const attribute = element.attributes[name];
  if (attribute === undefined) {
    failValidation(
      document,
      'WOML_MISSING_ATTRIBUTE',
      `<${element.name}> requires the "${name}" attribute.`,
      element.openTagSpan
    );
  }
  return attribute;
}

function optionalBoolean(
  document: WomlSourceDocument,
  element: WomlSourceElement,
  name: 'required' | 'secret'
): boolean {
  const attribute = element.attributes[name];
  if (attribute === undefined) return false;
  if (attribute.value !== 'true' && attribute.value !== 'false') {
    failValidation(
      document,
      'WOML_PROP_BOOLEAN_INVALID',
      `Prop attribute "${name}" must be exactly "true" or "false".`,
      attribute.valueSpan
    );
  }
  return attribute.value === 'true';
}

function propBindingName(name: string): string {
  return name.replace(/-([a-z0-9])/g, (_, value: string) =>
    value.toUpperCase()
  );
}

function validateProps(
  document: WomlSourceDocument,
  propsElement: WomlSourceElement | undefined
): readonly WomlPropDeclaration[] {
  if (propsElement === undefined) return [];
  ensureNoAttributes(document, propsElement);
  const children = elementChildren(document, propsElement);
  if (children.length === 0) {
    failValidation(
      document,
      'WOML_PROPS_EMPTY',
      '<props> requires at least one <prop> declaration.',
      propsElement.openTagSpan,
      'Remove the empty <props> block when the reusable definition has no props.'
    );
  }

  const names = new Set<string>();
  const bindings = new Set<string>();
  return children.map(child => {
    if (child.name !== 'prop') {
      failValidation(
        document,
        'WOML_PROPS_INVALID_CHILD',
        `<props> accepts <prop> declarations only; found <${child.name}>.`,
        child.openTagSpan
      );
    }
    ensureOnlyAttributes(
      document,
      child,
      new Set(['name', 'required', 'secret'])
    );
    if (child.children.length > 0) {
      failValidation(
        document,
        'WOML_PROP_NOT_EMPTY',
        '<prop> is an empty declaration and cannot contain child content.',
        child.children[0].span
      );
    }
    const nameAttribute = requiredAttribute(document, child, 'name');
    const name = nameAttribute.value;
    const bindingName = propBindingName(name);
    if (name.length > 128 || !kebabPattern.test(name)) {
      failValidation(
        document,
        'WOML_PROP_NAME_INVALID',
        `Prop name "${name}" must use lowercase kebab-case.`,
        nameAttribute.valueSpan,
        'Example: customer-id'
      );
    }
    if (reservedBindingNames.has(bindingName)) {
      failValidation(
        document,
        'WOML_PROP_NAME_RESERVED',
        `Prop name "${name}" maps to reserved JavaScript key "${bindingName}".`,
        nameAttribute.valueSpan
      );
    }
    if (names.has(name) || bindings.has(bindingName)) {
      failValidation(
        document,
        'WOML_PROP_DUPLICATE',
        `Prop "${name}" duplicates an existing source or JavaScript binding name.`,
        nameAttribute.valueSpan
      );
    }
    names.add(name);
    bindings.add(bindingName);
    return {
      name,
      bindingName,
      required: optionalBoolean(document, child, 'required'),
      secret: optionalBoolean(document, child, 'secret'),
      element: child,
    };
  });
}

function validateImports(
  document: WomlSourceDocument,
  imports: WomlSourceElement | undefined
): readonly WomlImportDeclaration[] {
  if (imports === undefined) return [];
  ensureNoAttributes(document, imports);
  const children = elementChildren(document, imports);
  if (children.length === 0) {
    failValidation(
      document,
      'WOML_IMPORTS_EMPTY',
      '<imports> requires at least one <module> declaration.',
      imports.openTagSpan
    );
  }
  const aliases = new Set<string>();
  const sources = new Set<string>();
  return children.map(child => {
    if (child.name !== 'module') {
      failValidation(
        document,
        'WOML_IMPORTS_INVALID_CHILD',
        `<imports> accepts <module> declarations only; found <${child.name}>.`,
        child.openTagSpan
      );
    }
    ensureOnlyAttributes(document, child, new Set(['name', 'from']));
    if (child.children.length > 0) {
      failValidation(
        document,
        'WOML_INVALID_STRUCTURE',
        '<module> cannot contain child content.',
        child.children[0].span
      );
    }
    const nameAttribute = requiredAttribute(document, child, 'name');
    const fromAttribute = requiredAttribute(document, child, 'from');
    const name = nameAttribute.value;
    const from = fromAttribute.value;
    const reusable = from.endsWith('.woml');
    const nameValid = reusable
      ? kebabPattern.test(name)
      : lowerCamelPattern.test(name);
    if (name.length > 128 || !nameValid) {
      failValidation(
        document,
        'WOML_MODULE_ALIAS_INVALID',
        reusable
          ? `Reusable definition name "${name}" must use lowercase kebab-case.`
          : `Module name "${name}" must be a JavaScript-safe lower-camel alias.`,
        nameAttribute.valueSpan,
        reusable ? 'Example: calculate-discount' : 'Example: spreadsheet or customerTools'
      );
    }
    if (reusable && reservedCustomTags.has(name)) {
      failValidation(
        document,
        'WOML_REUSABLE_ALIAS_RESERVED',
        `Reusable definition name "${name}" is reserved by WOML.`,
        nameAttribute.valueSpan
      );
    }
    if (aliases.has(name)) {
      failValidation(
        document,
        'WOML_MODULE_ALIAS_DUPLICATE',
        `Import name "${name}" is declared more than once.`,
        nameAttribute.valueSpan
      );
    }
    if (sources.has(from)) {
      failValidation(
        document,
        'WOML_MODULE_SOURCE_DUPLICATE',
        `Import source "${from}" is already declared under another name.`,
        fromAttribute.valueSpan
      );
    }
    const validSource = reusable
      ? relativeWomlPattern.test(from)
      : relativeScriptPattern.test(from);
    if (
      !validSource ||
      from.includes('\\') ||
      from.includes('\0') ||
      from.includes('?') ||
      from.includes('#')
    ) {
      failValidation(
        document,
        'WOML_MODULE_PATH_INVALID',
        `Import source "${from}" must be an explicit relative POSIX path ending in .js, .ts, or .woml.`,
        fromAttribute.valueSpan
      );
    }
    aliases.add(name);
    sources.add(from);
    return {
      name,
      from,
      kind: reusable ? 'reusable-definition' : 'script-module',
      element: child,
    };
  });
}

function findDescendant(
  element: WomlSourceElement,
  name: string
): WomlSourceElement | undefined {
  if (element.name === name) return element;
  for (const child of element.children) {
    if (child.kind !== 'element') continue;
    const found = findDescendant(child, name);
    if (found !== undefined) return found;
  }
  return undefined;
}

function validateScriptDefinition(
  document: WomlSourceDocument,
  definition: WomlSourceElement,
  kind: 'reusable-step' | 'notification-provider'
): void {
  if (kind === 'reusable-step') {
    ensureOnlyAttributes(document, definition, new Set(['name', 'description']));
  } else {
    ensureOnlyAttributes(document, definition, new Set(['kind']));
    const kindAttribute = requiredAttribute(document, definition, 'kind');
    if (kindAttribute.value !== 'notification') {
      failValidation(
        document,
        'WOML_PROVIDER_KIND_UNSUPPORTED',
        `Provider kind "${kindAttribute.value}" is not executable in this release.`,
        kindAttribute.valueSpan,
        kindAttribute.value === 'trigger'
          ? 'Custom trigger providers are reserved for a future reviewed inbound-provider contract.'
          : 'Use kind="notification".'
      );
    }
  }
  const children = elementChildren(document, definition);
  if (children.length !== 1 || children[0].name !== 'script') {
    failValidation(
      document,
      kind === 'reusable-step'
        ? 'WOML_REUSABLE_STEP_OPERATION_REQUIRED'
        : 'WOML_PROVIDER_OPERATION_REQUIRED',
      `<${definition.name}> must contain exactly one <script>.`,
      children[1]?.openTagSpan ?? children[0]?.openTagSpan ?? definition.openTagSpan
    );
  }
  ensureNoAttributes(document, children[0]);
  if (
    children[0].children.length !== 1 ||
    children[0].children[0].kind !== 'raw'
  ) {
    failValidation(
      document,
      'WOML_SCRIPT_BODY_REQUIRED',
      '<script> requires one raw JavaScript body.',
      children[0].openTagSpan
    );
  }
}

function validateReusableLifecycle(
  document: WomlSourceDocument,
  lifecycle: WomlSourceElement | undefined
): void {
  if (lifecycle === undefined) return;
  ensureNoAttributes(document, lifecycle);
  const hooks = elementChildren(document, lifecycle);
  if (hooks.length === 0) {
    failValidation(
      document,
      'WOML_LIFECYCLE_ACTION_REQUIRED',
      '<lifecycle> requires at least one reusable lifecycle hook.',
      lifecycle.openTagSpan
    );
  }
  const order = ['on-success', 'on-error', 'on-complete'];
  const seen = new Set<string>();
  for (const hook of hooks) {
    const index = order.indexOf(hook.name);
    if (index === -1) {
      failValidation(
        document,
        hook.name.startsWith('on-step-')
          ? 'WOML_REUSABLE_STEP_HOOK_UNSUPPORTED'
          : 'WOML_REUSABLE_LIFECYCLE_HOOK_INVALID',
        `<${hook.name}> is not available in a reusable definition lifecycle.`,
        hook.openTagSpan,
        'Use <on-success>, <on-error>, and <on-complete> only.'
      );
    }
    if (seen.has(hook.name)) {
      failValidation(
        document,
        'WOML_LIFECYCLE_DUPLICATE',
        `<${hook.name}> may appear only once.`,
        hook.openTagSpan
      );
    }
    ensureNoAttributes(document, hook);
    const actions = elementChildren(document, hook);
    if (actions.length === 0) {
      failValidation(
        document,
        'WOML_LIFECYCLE_ACTION_REQUIRED',
        `<${hook.name}> requires at least one <script> action.`,
        hook.openTagSpan
      );
    }
    for (const action of actions) {
      if (action.name === 'notify') {
        failValidation(
          document,
          'WOML_REUSABLE_LIFECYCLE_NOTIFY_UNSUPPORTED',
          'Reusable definition lifecycle hooks support script actions only in v1.',
          action.openTagSpan,
          'Use the custom provider from a workflow lifecycle <notify> action, where notification delivery is durable and recoverable.'
        );
      }
      if (action.name !== 'script') {
        failValidation(
          document,
          'WOML_LIFECYCLE_ACTION_INVALID',
          `<${hook.name}> supports <script> actions only.`,
          action.openTagSpan
        );
      }
    }
    seen.add(hook.name);
  }
}

export function inspectWomlDocument(
  document: WomlSourceDocument
): WomlDocumentInspection {
  const root = document.root;
  if (root.name !== 'woml') {
    failValidation(
      document,
      'WOML_EXPECTED_DOCUMENT_ROOT',
      `Expected <woml> as the document root, found <${root.name}>.`,
      root.openTagSpan
    );
  }
  ensureNoAttributes(document, root);
  const children = elementChildren(document, root);
  const workflow = children.find(child => child.name === 'workflow');
  const topLevelStep = children.find(child => child.name === 'step');
  const provider = children.find(child => child.name === 'provider');
  const executableCount = [workflow, topLevelStep, provider].filter(Boolean).length;
  if (executableCount !== 1) {
    if (
      executableCount === 0 &&
      children.every(child => child.name === 'imports' || child.name === 'props')
    ) {
      failValidation(
        document,
        'WOML_DOCUMENT_STRUCTURE_INVALID',
        '<woml> requires exactly one <workflow>, reusable <step>, or <provider> definition.',
        children[0]?.openTagSpan ?? root.openTagSpan
      );
    }
    failValidation(
      document,
      'WOML_DOCUMENT_PROFILE_INVALID',
      '<woml> must contain exactly one <workflow>, reusable <step>, or <provider> definition.',
      (workflow ?? topLevelStep ?? provider ?? children[0] ?? root).openTagSpan
    );
  }

  const imports = children[0]?.name === 'imports' ? children[0] : undefined;
  const props = children.find(child => child.name === 'props');
  const lifecycle = children.find(child => child.name === 'lifecycle');

  if (workflow !== undefined) {
    const forbiddenProps = findDescendant(root, 'props');
    if (forbiddenProps !== undefined) {
      failValidation(
        document,
        'WOML_PROPS_WORKFLOW_FORBIDDEN',
        '<props> is available only in reusable step or provider definition files, never in a workflow document.',
        forbiddenProps.openTagSpan
      );
    }
    const expected = [imports, workflow].filter(
      (value): value is WomlSourceElement => value !== undefined
    );
    if (
      children.length !== expected.length ||
      children.some((child, index) => child !== expected[index])
    ) {
      failValidation(
        document,
        'WOML_DOCUMENT_STRUCTURE_INVALID',
        '<woml> workflow documents require optional <imports> followed by exactly one <workflow>.',
        children.find((child, index) => child !== expected[index])?.openTagSpan ?? root.openTagSpan
      );
    }
    return {
      kind: 'workflow',
      sourcePath: document.file,
      imports: validateImports(document, imports),
      props: [],
      definition: workflow,
    };
  }

  const definition = topLevelStep ?? provider!;
  const expected = [imports, props, definition, lifecycle].filter(
    (value): value is WomlSourceElement => value !== undefined
  );
  if (
    children.length !== expected.length ||
    children.some((child, index) => child !== expected[index])
  ) {
    failValidation(
      document,
      'WOML_DOCUMENT_STRUCTURE_INVALID',
      'Reusable documents require optional <imports>, optional <props>, one <step> or <provider>, then optional <lifecycle>.',
      children.find((child, index) => child !== expected[index])?.openTagSpan ?? root.openTagSpan
    );
  }
  const kind = topLevelStep === undefined
    ? 'notification-provider' as const
    : 'reusable-step' as const;
  validateScriptDefinition(document, definition, kind);
  validateReusableLifecycle(document, lifecycle);
  return {
    kind,
    sourcePath: document.file,
    imports: validateImports(document, imports),
    props: validateProps(document, props),
    definition,
    ...(lifecycle === undefined ? {} : { lifecycle }),
  };
}

function validatePropValue(
  document: WomlSourceDocument,
  prop: WomlPropDeclaration,
  attribute: WomlSourceAttribute
): void {
  if (prop.secret) {
    if (!exactSecretReference.test(attribute.value)) {
      failValidation(
        document,
        'WOML_REUSABLE_SECRET_PROP_INVALID',
        `Secret prop "${prop.name}" requires exactly {{secrets.SECRET_NAME}}.`,
        attribute.valueSpan
      );
    }
    return;
  }
  if (exactSecretReference.test(attribute.value)) {
    failValidation(
      document,
      'WOML_REUSABLE_SECRET_PROP_UNDECLARED',
      `Prop "${prop.name}" is not declared secret="true".`,
      attribute.valueSpan
    );
  }
  const hasTemplateMarker = attribute.value.includes('{{') || attribute.value.includes('}}');
  if (hasTemplateMarker && !exactContextReference.test(attribute.value)) {
    failValidation(
      document,
      'WOML_REUSABLE_PROP_VALUE_INVALID',
      `Prop "${prop.name}" must be a literal string or one exact context reference.`,
      attribute.valueSpan,
      'Mixed interpolation is not supported in reusable props v1.'
    );
  }
}

function validateCustomTagUsage(
  document: WomlSourceDocument,
  element: WomlSourceElement,
  parent: WomlSourceElement | undefined,
  definitions: ReadonlyMap<string, WomlDocumentInspection>,
  ancestors: readonly WomlSourceElement[]
): void {
  const definition = definitions.get(element.name);
  if (definition === undefined) return;
  if (
    parent?.name === 'triggers' &&
    ['slack', 'telegram', 'discord', 'whatsapp'].includes(element.name)
  ) {
    return;
  }
  const stepParents = new Set([
    'steps',
    'when',
    'otherwise',
    'branch',
    'parallel',
    'for-each',
    'when-approved',
    'when-rejected',
    'case',
    'default',
  ]);
  if (definition.kind === 'reusable-step' && !stepParents.has(parent?.name ?? '')) {
    failValidation(
      document,
      'WOML_REUSABLE_STEP_PLACEMENT_INVALID',
      `Reusable step <${element.name}> is valid only where an ordinary <step> is allowed.`,
      element.openTagSpan
    );
  }
  if (definition.kind === 'notification-provider' && parent?.name !== 'notify') {
    failValidation(
      document,
      'WOML_REUSABLE_PROVIDER_PLACEMENT_INVALID',
      `Notification provider <${element.name}> is valid only as a direct child of <notify>.`,
      element.openTagSpan
    );
  }
  if (
    definition.kind === 'notification-provider' &&
    ancestors.some(ancestor => ancestor.name.startsWith('on-')) &&
    element.attributes.message === undefined
  ) {
    failValidation(
      document,
      'WOML_REUSABLE_PROVIDER_MESSAGE_REQUIRED',
      `Informational provider <${element.name}> requires the "message" attribute.`,
      element.openTagSpan
    );
  }
  ensureEmpty(document, element);
  const reserved = definition.kind === 'reusable-step'
    ? new Set([
        'id',
        'name',
        'description',
        'retry',
        'retry-delay',
        'retry-backoff',
        'retry-max-delay',
      ])
    : new Set(['message']);
  if (definition.kind === 'reusable-step') {
    const id = requiredAttribute(document, element, 'id');
    if (!lowerCamelPattern.test(id.value)) {
      failValidation(
        document,
        'WOML_INVALID_ID',
        `Custom step ID "${id.value}" must be a JavaScript-safe lower-camel identifier.`,
        id.valueSpan
      );
    }
  }
  const props = new Map(definition.props.map(prop => [prop.name, prop]));
  for (const attribute of Object.values(element.attributes)) {
    if (reserved.has(attribute.name)) continue;
    const prop = props.get(attribute.name);
    if (prop === undefined) {
      failValidation(
        document,
        'WOML_REUSABLE_PROP_UNKNOWN',
        `<${element.name}> does not declare prop "${attribute.name}".`,
        attribute.nameSpan
      );
    }
    validatePropValue(document, prop, attribute);
  }
  for (const prop of definition.props) {
    if (prop.required && element.attributes[prop.name] === undefined) {
      failValidation(
        document,
        'WOML_REUSABLE_PROP_REQUIRED',
        `<${element.name}> requires prop "${prop.name}".`,
        element.openTagSpan
      );
    }
  }
}

function walkCustomTagUsage(
  document: WomlSourceDocument,
  element: WomlSourceElement,
  definitions: ReadonlyMap<string, WomlDocumentInspection>,
  parent?: WomlSourceElement,
  ancestors: readonly WomlSourceElement[] = []
): void {
  validateCustomTagUsage(document, element, parent, definitions, ancestors);
  for (const child of element.children) {
    if (child.kind === 'element') {
      walkCustomTagUsage(
        document,
        child,
        definitions,
        element,
        [...ancestors, element]
      );
    }
  }
}

function portablePath(projectRoot: string, path: string): string {
  return relative(projectRoot, path).split(sep).join('/');
}

function sha256(value: string): string {
  return `sha256:${createHash('sha256').update(value).digest('hex')}`;
}

function failImportedModule(
  projectRoot: string,
  path: string,
  source: string,
  code: string,
  message: string,
  token?: string
): never {
  const file = new SourceFile(portablePath(projectRoot, path), source);
  const offset = token === undefined ? 0 : Math.max(0, source.indexOf(token));
  throw new WomlCompileError({
    phase: 'compile',
    code,
    message,
    file: file.path,
    location: file.span(offset, offset + Math.max(token?.length ?? 1, 1)),
  });
}

function resolveScriptModuleGraph(
  entryPath: string,
  projectRoot: string,
  sources: Map<string, {
    path: string;
    digest: string;
    kind: WomlDocumentKind | 'script-module';
  }>,
  stack: readonly string[] = []
): void {
  if (sources.has(entryPath)) return;
  let source: string;
  try {
    source = readFileSync(entryPath, 'utf8');
  } catch {
    failImportedModule(
      projectRoot,
      entryPath,
      '',
      'WOML_MODULE_FILE_NOT_READABLE',
      `Module source "${portablePath(projectRoot, entryPath)}" could not be read.`
    );
  }
  let imports: ReturnType<Bun.Transpiler['scan']>['imports'];
  try {
    const loader = entryPath.endsWith('.ts') ? 'ts' : 'js';
    imports = new Bun.Transpiler({ loader }).scan(source).imports;
  } catch (error) {
    failImportedModule(
      projectRoot,
      entryPath,
      source,
      'WOML_MODULE_SYNTAX_INVALID',
      `Cannot parse module source: ${error instanceof Error ? error.message : String(error)}`
    );
  }
  for (const imported of imports) {
    if (imported.kind !== 'import-statement') {
      failImportedModule(
        projectRoot,
        entryPath,
        source,
        imported.kind === 'dynamic-import'
          ? 'WOML_MODULE_DYNAMIC_IMPORT_UNSUPPORTED'
          : 'WOML_MODULE_IMPORT_UNSUPPORTED',
        `Import form "${imported.kind}" is unsupported in Module profile v1.`,
        imported.path
      );
    }
    if (!imported.path.startsWith('./') && !imported.path.startsWith('../')) {
      failImportedModule(
        projectRoot,
        entryPath,
        source,
        'WOML_MODULE_PACKAGE_IMPORT_UNAVAILABLE',
        `Package import "${imported.path}" is unavailable until the postponed package milestone.`,
        imported.path
      );
    }
    if (!/\.(?:js|ts)$/.test(imported.path)) {
      failImportedModule(
        projectRoot,
        entryPath,
        source,
        'WOML_MODULE_IMPORT_EXTENSION_REQUIRED',
        `Static import "${imported.path}" requires an explicit .js or .ts extension.`,
        imported.path
      );
    }
    const candidate = resolve(dirname(entryPath), imported.path);
    let target: string;
    try {
      target = realpathSync(candidate);
      if (!statSync(target).isFile()) throw new Error('not a file');
    } catch {
      failImportedModule(
        projectRoot,
        entryPath,
        source,
        'WOML_MODULE_FILE_NOT_FOUND',
        `Module dependency "${imported.path}" does not resolve to a readable file.`,
        imported.path
      );
    }
    const escaped = relative(projectRoot, target);
    if (escaped === '..' || escaped.startsWith(`..${sep}`) || isAbsolute(escaped)) {
      failImportedModule(
        projectRoot,
        entryPath,
        source,
        'WOML_MODULE_SYMLINK_ESCAPE',
        `Module dependency "${imported.path}" resolves outside the project boundary.`,
        imported.path
      );
    }
    if (stack.includes(target)) {
      failImportedModule(
        projectRoot,
        entryPath,
        source,
        'WOML_MODULE_GRAPH_CYCLE',
        `Module dependency graph contains a cycle through "${imported.path}".`,
        imported.path
      );
    }
    resolveScriptModuleGraph(target, projectRoot, sources, [...stack, entryPath]);
  }
  sources.set(entryPath, {
    path: portablePath(projectRoot, entryPath),
    digest: sha256(source),
    kind: 'script-module',
  });
}

function resolveInsideProject(
  importingDocument: WomlSourceDocument,
  declaration: WomlImportDeclaration,
  projectRoot: string
): string {
  const { from } = declaration;
  const candidate = resolve(dirname(importingDocument.file), from);
  let actual: string;
  try {
    const stats = statSync(candidate);
    if (!stats.isFile()) throw new Error('not a file');
    actual = realpathSync(candidate);
  } catch {
    failCompile(
      importingDocument,
      declaration.kind === 'reusable-definition'
        ? 'WOML_REUSABLE_SOURCE_NOT_FOUND'
        : 'WOML_MODULE_FILE_NOT_FOUND',
      `Import source "${from}" does not exist or is not a file.`,
      declaration.element.attributes.from?.valueSpan ?? declaration.element.openTagSpan
    );
  }
  const escaped = relative(projectRoot, actual);
  if (escaped === '..' || escaped.startsWith(`..${sep}`) || isAbsolute(escaped)) {
    failCompile(
      importingDocument,
      'WOML_MODULE_PROJECT_BOUNDARY',
      `Import source "${from}" resolves outside the project boundary.`,
      declaration.element.attributes.from?.valueSpan ?? declaration.element.openTagSpan
    );
  }
  return actual;
}

export function resolveWomlReusableDefinitionGraph(
  document: WomlSourceDocument,
  options: WomlReusableDefinitionResolverOptions = {}
): WomlReusableDefinitionGraph {
  const sourcePath = realpathSync(resolve(options.sourcePath ?? document.file));
  const projectRoot = realpathSync(resolve(options.projectRoot ?? dirname(sourcePath)));
  // The caller already parsed the root bytes. `sourcePath` supplies their
  // canonical identity; it must never cause us to silently replace those
  // bytes with another file while validating an in-memory/editor document.
  const rootDocument = { ...document, file: sourcePath };
  const root = inspectWomlDocument(rootDocument);
  const definitions: WomlResolvedReusableDefinition[] = [];
  const sources = new Map<string, {
    path: string;
    digest: string;
    kind: WomlDocumentKind | 'script-module';
  }>();
  sources.set(sourcePath, {
    path: portablePath(projectRoot, sourcePath),
    digest: sha256(rootDocument.source),
    kind: root.kind,
  });

  const resolveImports = (
    ownerDocument: WomlSourceDocument,
    owner: WomlDocumentInspection,
    stack: readonly string[]
  ): Map<string, WomlDocumentInspection> => {
    const direct = new Map<string, WomlDocumentInspection>();
    const canonicalTargets = new Set<string>();
    for (const declaration of owner.imports) {
      const target = resolveInsideProject(ownerDocument, declaration, projectRoot);
      if (canonicalTargets.has(target)) {
        failCompile(
          ownerDocument,
          'WOML_MODULE_SOURCE_DUPLICATE',
          `Import source "${declaration.from}" resolves to a file already imported by this document.`,
          declaration.element.attributes.from?.valueSpan ?? declaration.element.openTagSpan
        );
      }
      canonicalTargets.add(target);
      const source = readFileSync(target, 'utf8');
      if (declaration.kind === 'script-module') {
        resolveScriptModuleGraph(target, projectRoot, sources);
        continue;
      }
      if (stack.includes(target)) {
        failCompile(
          ownerDocument,
          'WOML_REUSABLE_IMPORT_CYCLE',
          `Reusable definition import cycle detected: ${[...stack, target]
            .map(path => portablePath(projectRoot, path))
            .join(' -> ')}.`,
          declaration.element.openTagSpan
        );
      }
      const targetDocument = parseWoml(source, { file: target });
      const targetInspection = inspectWomlDocument(targetDocument);
      if (targetInspection.kind === 'workflow') {
        failCompile(
          ownerDocument,
          'WOML_REUSABLE_WORKFLOW_IMPORT_FORBIDDEN',
          `Import "${declaration.name}" points to a runnable workflow, not a reusable step or provider.`,
          declaration.element.openTagSpan,
          'Use services.workflows.call() or services.workflows.start() for workflow-to-workflow communication.'
        );
      }
      direct.set(declaration.name, targetInspection);
      const digest = sha256(source);
      sources.set(target, {
        path: portablePath(projectRoot, target),
        digest,
        kind: targetInspection.kind,
      });
      definitions.push({
        alias: declaration.name,
        sourcePath: portablePath(projectRoot, target),
        digest,
        kind: targetInspection.kind,
        props: targetInspection.props.map(({ element: _element, ...prop }) => prop),
      });
      const nested = resolveImports(targetDocument, targetInspection, [...stack, target]);
      walkCustomTagUsage(targetDocument, targetDocument.root, nested);
    }
    return direct;
  };

  const direct = resolveImports(rootDocument, root, [sourcePath]);
  walkCustomTagUsage(rootDocument, rootDocument.root, direct);
  const orderedSources = [...sources.values()].sort((left, right) =>
    left.path.localeCompare(right.path)
  );
  const orderedDefinitions = [...definitions].sort((left, right) =>
    `${left.alias}\0${left.sourcePath}`.localeCompare(`${right.alias}\0${right.sourcePath}`)
  );
  const rootHash = sha256(
    JSON.stringify({
      root: portablePath(projectRoot, sourcePath),
      sources: orderedSources,
      definitions: orderedDefinitions,
    })
  );
  return { root, definitions: orderedDefinitions, sources: orderedSources, rootHash };
}

export function assertWomlDocumentRunnable(
  document: WomlSourceDocument
): void {
  const inspection = inspectWomlDocument(document);
  if (inspection.kind !== 'workflow') {
    failCompile(
      document,
      'WOML_DEFINITION_NOT_RUNNABLE',
      `This file defines a ${inspection.kind === 'reusable-step' ? 'reusable step' : 'notification provider'} and cannot be run independently.`,
      inspection.definition.openTagSpan,
      'Import it from a workflow with <module name="custom-tag" from="./definition.woml" />.'
    );
  }
}

export function generateWomlReusableCustomData(
  graph: WomlReusableDefinitionGraph
): string {
  const tags = graph.definitions.map(definition => ({
    name: definition.alias,
    description:
      definition.kind === 'reusable-step'
        ? `Reusable WOML step from ${definition.sourcePath}`
        : `Reusable WOML notification provider from ${definition.sourcePath}`,
    attributes: [
      ...(definition.kind === 'reusable-step'
        ? [
            { name: 'id', description: 'Required workflow-wide step identity.' },
            { name: 'name', description: 'Optional invocation display name.' },
            { name: 'description', description: 'Optional invocation description.' },
            { name: 'retry', description: 'Optional total attempt count.' },
          ]
        : [{ name: 'message', description: 'Required for informational lifecycle notifications.' }]),
      ...definition.props.map(prop => ({
        name: prop.name,
        description: `${prop.required ? 'Required' : 'Optional'}${prop.secret ? ' secret' : ''} prop; available as props.${prop.bindingName}.`,
      })),
    ],
  }));
  return `${JSON.stringify({ version: 1.1, tags }, null, 2)}\n`;
}
