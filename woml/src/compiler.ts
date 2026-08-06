import {
  inspectCompiledWorkflowGraph,
  type CompiledWorkflowDefinition,
  type CompiledWorkflowEdge,
  type CompiledWorkflowGraph,
  type CompiledWorkflowMetadata,
  type CompiledWorkflowNode,
  type ContextReferenceExpression,
  type JsonValue,
} from './model';
import {
  SourceFile,
  WomlCompileError,
  WomlValidationError,
  type SourceSpan,
  type WomlDiagnostic,
  type WomlSourceAttribute,
  type WomlSourceDocument,
  type WomlSourceElement,
  type WomlSourceRawText,
} from './source';

interface ElementProfile {
  readonly attributes: ReadonlySet<string>;
  readonly stagedAttributes?: ReadonlySet<string>;
}

interface ValidatedStep {
  readonly kind: 'step';
  readonly id: string;
  readonly source: string;
  readonly metadata?: Readonly<Record<string, JsonValue>>;
}

interface ValidatedReference {
  readonly path: readonly string[];
  readonly structuralId?: string;
  readonly span: SourceSpan;
}

interface ValidatedBranchArm {
  readonly kind: 'when' | 'otherwise';
  readonly element: WomlSourceElement;
  readonly test?: ValidatedReference;
  readonly items: readonly ValidatedFlowItem[];
  readonly result: ValidatedReference;
}

interface ValidatedBranch {
  readonly kind: 'branch';
  readonly id: string;
  readonly element: WomlSourceElement;
  readonly metadata?: Readonly<Record<string, JsonValue>>;
  readonly arms: readonly ValidatedBranchArm[];
}

interface ValidatedParallel {
  readonly kind: 'parallel';
  readonly id: string;
  readonly element: WomlSourceElement;
  readonly metadata?: Readonly<Record<string, JsonValue>>;
  readonly concurrency: number;
  readonly onError: 'fail-fast' | 'wait-all';
  readonly children: readonly ValidatedStep[];
}

type ValidatedFlowItem = ValidatedStep | ValidatedBranch | ValidatedParallel;

interface ValidatedFlow {
  readonly items: readonly ValidatedFlowItem[];
  readonly firstBranch?: WomlSourceElement;
  readonly firstParallel?: WomlSourceElement;
}

interface LoweredFlowFragment {
  readonly entryId: string;
  readonly exitId: string;
  readonly nodes: readonly CompiledWorkflowNode[];
  readonly edges: readonly CompiledWorkflowEdge[];
}

const supportedElements = new Set([
  'workflow',
  'triggers',
  'manual',
  'steps',
  'step',
  'script',
  'branch',
  'parallel',
  'when',
  'otherwise',
  'result',
]);

const stagedElements = new Set([
  'config',
  'lifecycle',
  'on-success',
  'on-failure',
  'webhook',
  'schema',
  'schedule',
  'interval',
  'event',
  'approval',
  'notify',
  'when-approved',
  'when-rejected',
]);

const elementProfiles: Readonly<Record<string, ElementProfile>> = {
  workflow: {
    attributes: new Set(['id', 'name', 'description', 'version']),
    stagedAttributes: new Set(['tags']),
  },
  triggers: { attributes: new Set() },
  manual: { attributes: new Set(['id']) },
  steps: { attributes: new Set() },
  step: {
    attributes: new Set(['id', 'name', 'description']),
    stagedAttributes: new Set(['retry', 'timeout']),
  },
  script: { attributes: new Set() },
  branch: { attributes: new Set(['id', 'name', 'description']) },
  parallel: {
    attributes: new Set([
      'id',
      'name',
      'description',
      'concurrency',
      'on-error',
    ]),
  },
  when: { attributes: new Set(['test']) },
  otherwise: { attributes: new Set() },
  result: { attributes: new Set(['value']) },
};

const workflowIdPattern = /^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$/;
const javascriptSafeIdPattern = /^[a-z][A-Za-z0-9]*$/;

function diagnostic(
  document: WomlSourceDocument,
  phase: 'validation' | 'compile',
  code: string,
  message: string,
  span: SourceSpan,
  hint?: string
): WomlDiagnostic {
  return {
    code,
    phase,
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
  span: SourceSpan
): never {
  throw new WomlCompileError(
    diagnostic(document, 'compile', code, message, span)
  );
}

function visitProfile(
  document: WomlSourceDocument,
  element: WomlSourceElement
): void {
  if (!supportedElements.has(element.name)) {
    if (stagedElements.has(element.name)) {
      failValidation(
        document,
        'WOML_FEATURE_NOT_EXECUTABLE',
        `<${element.name}> is designed but is not executable in the first WOML CLI profile.`,
        element.openTagSpan
      );
    }
    failValidation(
      document,
      'WOML_UNKNOWN_ELEMENT',
      `Unknown WOML element <${element.name}>.`,
      element.openTagSpan
    );
  }

  const profile = elementProfiles[element.name];
  for (const attribute of Object.values(element.attributes)) {
    if (profile.stagedAttributes?.has(attribute.name) === true) {
      failValidation(
        document,
        'WOML_FEATURE_NOT_EXECUTABLE',
        `Attribute "${attribute.name}" on <${element.name}> is designed but is not executable in the first WOML CLI profile.`,
        attribute.nameSpan
      );
    }
    if (!profile.attributes.has(attribute.name)) {
      failValidation(
        document,
        'WOML_UNKNOWN_ATTRIBUTE',
        `Unknown attribute "${attribute.name}" on <${element.name}>.`,
        attribute.nameSpan
      );
    }
  }

  for (const child of element.children) {
    if (child.kind === 'element') visitProfile(document, child);
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

function elementChildren(
  document: WomlSourceDocument,
  parent: WomlSourceElement
): WomlSourceElement[] {
  const elements: WomlSourceElement[] = [];
  for (const child of parent.children) {
    if (child.kind !== 'element') {
      failValidation(
        document,
        'WOML_UNEXPECTED_CONTENT',
        `<${parent.name}> may contain WOML elements only.`,
        child.span
      );
    }
    elements.push(child);
  }
  return elements;
}

function ensureEmptyElement(
  document: WomlSourceDocument,
  element: WomlSourceElement
): void {
  if (element.children.length > 0) {
    failValidation(
      document,
      'WOML_INVALID_STRUCTURE',
      `<${element.name}> cannot contain child content in the first WOML CLI profile.`,
      element.children[0].span
    );
  }
}

function validateWorkflowId(
  document: WomlSourceDocument,
  attribute: WomlSourceAttribute
): string {
  if (
    attribute.value.length > 256 ||
    !workflowIdPattern.test(attribute.value)
  ) {
    failValidation(
      document,
      'WOML_INVALID_ID',
      `Workflow ID "${attribute.value}" must use lowercase kebab-case.`,
      attribute.valueSpan,
      'Example: content-moderator'
    );
  }
  return attribute.value;
}

function validateJavaScriptSafeId(
  document: WomlSourceDocument,
  attribute: WomlSourceAttribute,
  role: 'trigger' | 'step' | 'branch' | 'parallel'
): string {
  if (
    attribute.value.length > 256 ||
    !javascriptSafeIdPattern.test(attribute.value)
  ) {
    failValidation(
      document,
      'WOML_INVALID_ID',
      `${role === 'trigger' ? 'Trigger' : role === 'branch' ? 'Branch' : role === 'parallel' ? 'Parallel' : 'Step'} ID "${attribute.value}" must be a JavaScript-safe lower-camel identifier.`,
      attribute.valueSpan,
      'Use letters and numbers, start with a lowercase letter, and do not use hyphens.'
    );
  }
  return attribute.value;
}

function optionalMetadataValue(
  document: WomlSourceDocument,
  element: WomlSourceElement,
  name: 'name' | 'description' | 'version'
): string | undefined {
  const attribute = element.attributes[name];
  if (attribute === undefined) return undefined;
  if (attribute.value.trim().length === 0) {
    failValidation(
      document,
      'WOML_EMPTY_METADATA',
      `Attribute "${name}" on <${element.name}> must not be empty.`,
      attribute.valueSpan
    );
  }
  return attribute.value;
}

function workflowMetadata(
  document: WomlSourceDocument,
  workflow: WomlSourceElement
): CompiledWorkflowMetadata | undefined {
  const name = optionalMetadataValue(document, workflow, 'name');
  const description = optionalMetadataValue(document, workflow, 'description');
  const version = optionalMetadataValue(document, workflow, 'version');
  if (
    name === undefined &&
    description === undefined &&
    version === undefined
  ) {
    return undefined;
  }
  return {
    ...(name === undefined ? {} : { name }),
    ...(description === undefined ? {} : { description }),
    ...(version === undefined ? {} : { version }),
  };
}

function flowItemMetadata(
  document: WomlSourceDocument,
  element: WomlSourceElement
): Readonly<Record<string, JsonValue>> | undefined {
  const name = optionalMetadataValue(document, element, 'name');
  const description = optionalMetadataValue(document, element, 'description');
  if (name === undefined && description === undefined) return undefined;
  return {
    ...(name === undefined ? {} : { name }),
    ...(description === undefined ? {} : { description }),
  };
}

function validateWorkflowChildren(
  document: WomlSourceDocument,
  workflow: WomlSourceElement
): readonly [WomlSourceElement, WomlSourceElement] {
  const children = elementChildren(document, workflow);
  const triggerContainers = children.filter(child => child.name === 'triggers');
  const stepsContainers = children.filter(child => child.name === 'steps');

  if (triggerContainers.length !== 1) {
    failValidation(
      document,
      'WOML_TRIGGER_CONTAINER_COUNT',
      `<workflow> requires exactly one <triggers> container; found ${triggerContainers.length}.`,
      triggerContainers[1]?.openTagSpan ?? workflow.openTagSpan
    );
  }
  if (stepsContainers.length !== 1) {
    failValidation(
      document,
      'WOML_STEPS_CONTAINER_COUNT',
      `<workflow> requires exactly one <steps> container; found ${stepsContainers.length}.`,
      stepsContainers[1]?.openTagSpan ?? workflow.openTagSpan
    );
  }
  if (
    children.length !== 2 ||
    children[0] !== triggerContainers[0] ||
    children[1] !== stepsContainers[0]
  ) {
    const offender = children.find(
      (child, index) =>
        (index === 0 && child.name !== 'triggers') ||
        (index === 1 && child.name !== 'steps') ||
        index > 1
    );
    failValidation(
      document,
      'WOML_INVALID_STRUCTURE',
      '<workflow> must contain <triggers> followed by <steps>, with no other executable-profile children.',
      offender?.openTagSpan ?? workflow.openTagSpan
    );
  }

  return [triggerContainers[0], stepsContainers[0]];
}

function validateManualTrigger(
  document: WomlSourceDocument,
  triggers: WomlSourceElement
): string {
  const children = elementChildren(document, triggers);
  if (children.length !== 1 || children[0].name !== 'manual') {
    const found =
      children.length === 0
        ? 'none'
        : children.map(child => `<${child.name}>`).join(', ');
    failValidation(
      document,
      'WOML_MANUAL_TRIGGER_COUNT',
      `The first WOML CLI profile requires exactly one <manual> trigger; found ${found}.`,
      children[1]?.openTagSpan ??
        children[0]?.openTagSpan ??
        triggers.openTagSpan
    );
  }

  const manual = children[0];
  ensureEmptyElement(document, manual);
  return validateJavaScriptSafeId(
    document,
    requiredAttribute(document, manual, 'id'),
    'trigger'
  );
}

function scriptSource(
  document: WomlSourceDocument,
  script: WomlSourceElement
): string {
  const rawBodies = script.children.filter(
    (child): child is WomlSourceRawText => child.kind === 'raw'
  );
  if (
    script.children.some(child => child.kind !== 'raw') ||
    rawBodies.length > 1
  ) {
    failValidation(
      document,
      'WOML_INVALID_STRUCTURE',
      '<script> must contain one raw JavaScript body and no WOML child elements.',
      script.children[0]?.span ?? script.openTagSpan
    );
  }
  return rawBodies[0]?.value ?? '';
}

function referenceSpan(
  document: WomlSourceDocument,
  attribute: WomlSourceAttribute,
  structuralId: string
): SourceSpan {
  const relativeOffset = attribute.value.indexOf(
    structuralId,
    '{{context.steps.'.length
  );
  const start = attribute.valueSpan.start.offset + relativeOffset;
  return new SourceFile(document.file, document.source).span(
    start,
    start + structuralId.length
  );
}

function parseExactReference(
  document: WomlSourceDocument,
  attribute: WomlSourceAttribute
): ValidatedReference {
  const match =
    /^\{\{(context\.(?:trigger(?:\.[A-Za-z_$][A-Za-z0-9_$]*)*|steps\.([a-z][A-Za-z0-9]*)(?:\.[A-Za-z_$][A-Za-z0-9_$]*)*))\}\}$/.exec(
      attribute.value
    );
  if (match === null) {
    failValidation(
      document,
      'WOML_INVALID_REFERENCE',
      `Attribute "${attribute.name}" must contain exactly one WOML context reference.`,
      attribute.valueSpan,
      'Example: {{context.steps.checkContent.needsReview}}'
    );
  }

  const structuralId = match[2];
  const span =
    structuralId === undefined
      ? attribute.valueSpan
      : referenceSpan(document, attribute, structuralId);
  return {
    path: match[1].split('.').slice(1),
    ...(structuralId === undefined ? {} : { structuralId }),
    span,
  };
}

function registerStructuralId(
  document: WomlSourceDocument,
  registry: Set<string>,
  attribute: WomlSourceAttribute,
  role: 'step' | 'branch' | 'parallel'
): string {
  const id = validateJavaScriptSafeId(document, attribute, role);
  if (registry.has(id)) {
    failValidation(
      document,
      'WOML_DUPLICATE_ID',
      `Structural ID "${id}" is duplicated across workflow steps, branches, and parallel groups.`,
      attribute.valueSpan
    );
  }
  registry.add(id);
  return id;
}

function validateStep(
  document: WomlSourceDocument,
  step: WomlSourceElement,
  registry: Set<string>
): ValidatedStep {
  const id = registerStructuralId(
    document,
    registry,
    requiredAttribute(document, step, 'id'),
    'step'
  );

  const operations = elementChildren(document, step);
  if (operations.length !== 1 || operations[0].name !== 'script') {
    failValidation(
      document,
      'WOML_STEP_OPERATION_COUNT',
      `<step id="${id}"> must contain exactly one <script> operation.`,
      operations[1]?.openTagSpan ??
        operations[0]?.openTagSpan ??
        step.openTagSpan
    );
  }

  return {
    kind: 'step',
    id,
    source: scriptSource(document, operations[0]),
    metadata: flowItemMetadata(document, step),
  };
}

function validateBranchArm(
  document: WomlSourceDocument,
  arm: WomlSourceElement,
  registry: Set<string>
): ValidatedBranchArm {
  const testReference =
    arm.name === 'when'
      ? parseExactReference(document, requiredAttribute(document, arm, 'test'))
      : undefined;
  const children = elementChildren(document, arm);
  const results = children.filter(child => child.name === 'result');
  if (results.length !== 1) {
    failValidation(
      document,
      'WOML_BRANCH_RESULT_REQUIRED',
      `<${arm.name}> must contain exactly one <result>.`,
      results[1]?.openTagSpan ?? arm.openTagSpan
    );
  }

  const result = results[0];
  if (children.at(-1) !== result) {
    failValidation(
      document,
      'WOML_BRANCH_RESULT_ORDER',
      '<result> must be the final child of its branch arm.',
      result.openTagSpan
    );
  }
  ensureEmptyElement(document, result);

  const flowChildren = children.slice(0, -1);
  if (flowChildren.length === 0) {
    failValidation(
      document,
      'WOML_INVALID_STRUCTURE',
      `<${arm.name}> must contain at least one step item before <result>.`,
      arm.openTagSpan
    );
  }

  const items = flowChildren.map(child =>
    validateFlowItem(document, child, registry, `<${arm.name}>`)
  );
  const resultReference = parseExactReference(
    document,
    requiredAttribute(document, result, 'value')
  );

  if (arm.name === 'when') {
    return {
      kind: 'when',
      element: arm,
      test: testReference,
      items,
      result: resultReference,
    };
  }
  return {
    kind: 'otherwise',
    element: arm,
    items,
    result: resultReference,
  };
}

function validateBranch(
  document: WomlSourceDocument,
  branch: WomlSourceElement,
  registry: Set<string>
): ValidatedBranch {
  const id = registerStructuralId(
    document,
    registry,
    requiredAttribute(document, branch, 'id'),
    'branch'
  );
  const metadata = flowItemMetadata(document, branch);

  const children = elementChildren(document, branch);
  const whenCount = children.filter(child => child.name === 'when').length;
  if (whenCount === 0) {
    failValidation(
      document,
      'WOML_BRANCH_WHEN_REQUIRED',
      `<branch id="${id}"> requires at least one <when>.`,
      branch.openTagSpan
    );
  }

  const otherwiseIndexes = children.flatMap((child, index) =>
    child.name === 'otherwise' ? [index] : []
  );
  if (otherwiseIndexes.length === 0) {
    failValidation(
      document,
      'WOML_BRANCH_OTHERWISE_REQUIRED',
      `<branch id="${id}"> requires exactly one final <otherwise>.`,
      branch.openTagSpan
    );
  }
  if (otherwiseIndexes.length > 1) {
    failValidation(
      document,
      'WOML_BRANCH_OTHERWISE_ORDER',
      '<otherwise> may appear exactly once and must be the final branch case.',
      children[otherwiseIndexes[1]].openTagSpan
    );
  }
  const otherwiseIndex = otherwiseIndexes[0];
  if (otherwiseIndex !== children.length - 1) {
    failValidation(
      document,
      'WOML_BRANCH_OTHERWISE_ORDER',
      '<otherwise> must be the final child of <branch>.',
      children[otherwiseIndex].openTagSpan
    );
  }

  for (let index = 0; index < otherwiseIndex; index += 1) {
    if (children[index].name !== 'when') {
      failValidation(
        document,
        'WOML_INVALID_STRUCTURE',
        `<branch> cannot contain <${children[index].name}> outside a <when> or <otherwise> arm.`,
        children[index].openTagSpan
      );
    }
  }

  return {
    kind: 'branch',
    id,
    element: branch,
    metadata,
    arms: children.map(arm => validateBranchArm(document, arm, registry)),
  };
}

function validateParallel(
  document: WomlSourceDocument,
  parallel: WomlSourceElement,
  registry: Set<string>
): ValidatedParallel {
  const id = registerStructuralId(
    document,
    registry,
    requiredAttribute(document, parallel, 'id'),
    'parallel'
  );
  const metadata = flowItemMetadata(document, parallel);
  const childElements = elementChildren(document, parallel);

  if (childElements.length === 0) {
    failValidation(
      document,
      'WOML_PARALLEL_EMPTY',
      `<parallel id="${id}"> must contain at least one direct <step> child.`,
      parallel.openTagSpan
    );
  }

  const unsupportedChild = childElements.find(child => child.name !== 'step');
  if (unsupportedChild !== undefined) {
    failValidation(
      document,
      'WOML_PARALLEL_CHILD_UNSUPPORTED',
      `<parallel id="${id}"> accepts direct <step> children only in this profile; found <${unsupportedChild.name}>.`,
      unsupportedChild.openTagSpan,
      'Move branching around the parallel group, or wait for a future explicit lane/sequence construct.'
    );
  }

  const children = childElements.map(child =>
    validateStep(document, child, registry)
  );
  const concurrencyAttribute = parallel.attributes.concurrency;
  let concurrency = children.length;
  if (concurrencyAttribute !== undefined) {
    if (!/^[1-9][0-9]*$/.test(concurrencyAttribute.value)) {
      failValidation(
        document,
        'WOML_PARALLEL_INVALID_CONCURRENCY',
        `Parallel concurrency "${concurrencyAttribute.value}" must be a positive integer.`,
        concurrencyAttribute.valueSpan
      );
    }
    concurrency = Number(concurrencyAttribute.value);
    if (!Number.isSafeInteger(concurrency) || concurrency > children.length) {
      failValidation(
        document,
        'WOML_PARALLEL_INVALID_CONCURRENCY',
        `Parallel concurrency must not exceed its ${children.length} direct child${children.length === 1 ? '' : 'ren'}.`,
        concurrencyAttribute.valueSpan
      );
    }
  }

  const onErrorAttribute = parallel.attributes['on-error'];
  const onError = onErrorAttribute?.value ?? 'fail-fast';
  if (onError !== 'fail-fast' && onError !== 'wait-all') {
    failValidation(
      document,
      'WOML_PARALLEL_INVALID_POLICY',
      `Parallel on-error must be "fail-fast" or "wait-all", found "${onError}".`,
      onErrorAttribute!.valueSpan
    );
  }

  return {
    kind: 'parallel',
    id,
    element: parallel,
    metadata,
    concurrency,
    onError,
    children,
  };
}

function validateFlowItem(
  document: WomlSourceDocument,
  element: WomlSourceElement,
  registry: Set<string>,
  parent: string
): ValidatedFlowItem {
  if (element.name === 'step') return validateStep(document, element, registry);
  if (element.name === 'branch') {
    return validateBranch(document, element, registry);
  }
  if (element.name === 'parallel') {
    return validateParallel(document, element, registry);
  }
  failValidation(
    document,
    'WOML_INVALID_STRUCTURE',
    `${parent} cannot contain <${element.name}> as a flow item.`,
    element.openTagSpan
  );
}

function assertReferenceAvailable(
  document: WomlSourceDocument,
  reference: ValidatedReference,
  allIds: ReadonlySet<string>,
  availableIds: ReadonlySet<string>
): void {
  const id = reference.structuralId;
  if (id === undefined) return;
  if (!allIds.has(id)) {
    failCompile(
      document,
      'WOML_UNKNOWN_REFERENCE',
      `Reference names unknown structural ID "${id}".`,
      reference.span
    );
  }
  if (!availableIds.has(id)) {
    failCompile(
      document,
      'WOML_REFERENCE_NOT_DOMINATING',
      `Output "${id}" is not guaranteed to be available at this reference.`,
      reference.span
    );
  }
}

function validateReferenceAvailability(
  document: WomlSourceDocument,
  items: readonly ValidatedFlowItem[],
  allIds: ReadonlySet<string>,
  availableBefore: ReadonlySet<string> = new Set()
): Set<string> {
  const available = new Set(availableBefore);
  for (const item of items) {
    if (item.kind === 'step') {
      available.add(item.id);
      continue;
    }

    if (item.kind === 'parallel') {
      for (const child of item.children) available.add(child.id);
      continue;
    }

    for (const arm of item.arms) {
      if (arm.test !== undefined) {
        assertReferenceAvailable(document, arm.test, allIds, available);
      }
      const armAvailable = validateReferenceAvailability(
        document,
        arm.items,
        allIds,
        available
      );
      assertReferenceAvailable(document, arm.result, allIds, armAvailable);
    }
    available.add(item.id);
  }
  return available;
}

function validateSteps(
  document: WomlSourceDocument,
  stepsElement: WomlSourceElement
): ValidatedFlow {
  const children = elementChildren(document, stepsElement);
  if (children.length === 0) {
    failValidation(
      document,
      'WOML_EMPTY_STEPS',
      '<steps> must contain at least one step item.',
      stepsElement.openTagSpan
    );
  }

  const structuralIds = new Set<string>();
  const items = children.map(child =>
    validateFlowItem(document, child, structuralIds, '<steps>')
  );
  validateReferenceAvailability(document, items, structuralIds);

  const terminal = items.at(-1);
  if (terminal?.kind === 'parallel') {
    failValidation(
      document,
      'WOML_PARALLEL_TERMINAL_UNSUPPORTED',
      `Root <parallel id="${terminal.id}"> cannot be the final workflow item because a parallel group has no aggregate result.`,
      terminal.element.openTagSpan,
      'Add a downstream <step> that builds the workflow result.'
    );
  }

  const findFirstBranch = (
    flowItems: readonly ValidatedFlowItem[]
  ): WomlSourceElement | undefined => {
    for (const item of flowItems) {
      if (item.kind === 'branch') {
        return item.element;
      }
    }
    return undefined;
  };
  const findFirstParallel = (
    flowItems: readonly ValidatedFlowItem[]
  ): WomlSourceElement | undefined => {
    for (const item of flowItems) {
      if (item.kind === 'parallel') return item.element;
      if (item.kind === 'branch') {
        for (const arm of item.arms) {
          const nested = findFirstParallel(arm.items);
          if (nested !== undefined) return nested;
        }
      }
    }
    return undefined;
  };
  const firstBranch = findFirstBranch(items);
  const firstParallel = findFirstParallel(items);

  return {
    items,
    ...(firstBranch === undefined ? {} : { firstBranch }),
    ...(firstParallel === undefined ? {} : { firstParallel }),
  };
}

function lowerStep(step: ValidatedStep): LoweredFlowFragment {
  const node: CompiledWorkflowNode = {
    id: step.id,
    handler: 'runtime.script',
    ...(step.metadata === undefined ? {} : { metadata: step.metadata }),
    inputs: {
      kind: 'object',
      fields: {
        source: { kind: 'literal', value: step.source },
      },
    },
  };
  return {
    entryId: step.id,
    exitId: step.id,
    nodes: [node],
    edges: [],
  };
}

function referenceExpression(
  reference: ValidatedReference
): ContextReferenceExpression {
  return { kind: 'contextReference', path: reference.path };
}

function alwaysEdge(from: string, to: string): CompiledWorkflowEdge {
  return {
    id: `${from}-to-${to}`,
    from,
    to,
    condition: { kind: 'always' },
  };
}

function lowerBranch(branch: ValidatedBranch): LoweredFlowFragment {
  const selectorId = `__woml_branch__${branch.id}__select`;
  const armFragments = branch.arms.map(arm => lowerFlowItems(arm.items));
  const armIds = branch.arms.map((arm, index) =>
    arm.kind === 'when'
      ? `${branch.id}:when:${index}`
      : `${branch.id}:otherwise`
  );

  const selector: CompiledWorkflowNode = {
    id: selectorId,
    handler: 'engine.branch-select',
    inputs: { kind: 'object', fields: {} },
    ...(branch.metadata === undefined ? {} : { metadata: branch.metadata }),
  };
  const result: CompiledWorkflowNode = {
    id: branch.id,
    handler: 'engine.branch-result',
    inputs: {
      kind: 'object',
      fields: Object.fromEntries(
        branch.arms.map((arm, index) => [
          armIds[index],
          referenceExpression(arm.result),
        ])
      ),
    },
  };

  const selectionEdges: CompiledWorkflowEdge[] = branch.arms.map(
    (arm, index) => ({
      id: armIds[index],
      from: selectorId,
      to: armFragments[index].entryId,
      condition:
        arm.kind === 'when'
          ? {
              kind: 'boolean',
              value: referenceExpression(arm.test!),
            }
          : { kind: 'always' },
      branchId: branch.id,
    })
  );

  return {
    entryId: selectorId,
    exitId: branch.id,
    nodes: [
      selector,
      ...armFragments.flatMap(fragment => fragment.nodes),
      result,
    ],
    edges: [
      ...selectionEdges,
      ...armFragments.flatMap(fragment => fragment.edges),
      ...armFragments.map(fragment => alwaysEdge(fragment.exitId, branch.id)),
    ],
  };
}

function lowerFlowItem(item: ValidatedFlowItem): LoweredFlowFragment {
  if (item.kind === 'step') return lowerStep(item);
  if (item.kind === 'branch') return lowerBranch(item);
  throw new Error('Validated parallel lowering is gated until phase P2.');
}

function lowerFlowItems(
  items: readonly ValidatedFlowItem[]
): LoweredFlowFragment {
  const fragments = items.map(lowerFlowItem);
  const nodes: CompiledWorkflowNode[] = [];
  const edges: CompiledWorkflowEdge[] = [];

  for (let index = 0; index < fragments.length; index += 1) {
    const fragment = fragments[index];
    nodes.push(...fragment.nodes);
    if (index > 0) {
      edges.push(alwaysEdge(fragments[index - 1].exitId, fragment.entryId));
    }
    edges.push(...fragment.edges);
  }

  return {
    entryId: fragments[0].entryId,
    exitId: fragments.at(-1)!.exitId,
    nodes,
    edges,
  };
}

export function compileWoml(
  document: WomlSourceDocument
): CompiledWorkflowDefinition {
  const workflow = document.root;
  if (workflow.name !== 'workflow') {
    failValidation(
      document,
      'WOML_EXPECTED_WORKFLOW_ROOT',
      `Expected <workflow> as the document root, found <${workflow.name}>.`,
      workflow.openTagSpan
    );
  }

  visitProfile(document, workflow);

  const workflowId = validateWorkflowId(
    document,
    requiredAttribute(document, workflow, 'id')
  );
  const metadata = workflowMetadata(document, workflow);
  const [triggersElement, stepsElement] = validateWorkflowChildren(
    document,
    workflow
  );
  const triggerId = validateManualTrigger(document, triggersElement);
  const flow = validateSteps(document, stepsElement);
  if (flow.firstParallel !== undefined) {
    failCompile(
      document,
      'WOML_PARALLEL_LOWERING_NOT_IMPLEMENTED',
      'This parallel group is valid, but parallel DAG lowering is introduced in phase P2.',
      flow.firstParallel.openTagSpan
    );
  }
  const lowered = lowerFlowItems(flow.items);
  const definition = {
    workflowId,
    ...(metadata === undefined ? {} : { metadata }),
    triggers: [
      {
        id: triggerId,
        handler: 'trigger.manual',
        config: { kind: 'object' as const, fields: {} },
      },
    ],
    graph: {
      entryNodeIds: [lowered.entryId],
      nodes: lowered.nodes,
      edges: lowered.edges,
    } satisfies CompiledWorkflowGraph,
  };
  const compiled: CompiledWorkflowDefinition =
    flow.firstBranch === undefined
      ? { schemaVersion: 1, ...definition }
      : { schemaVersion: 2, ...definition };

  const graphIssues = inspectCompiledWorkflowGraph(compiled.graph, {
    requireSingleTerminal: true,
  });
  if (graphIssues.length > 0) {
    failCompile(
      document,
      'WOML_INVALID_DAG',
      graphIssues[0].message,
      workflow.openTagSpan
    );
  }

  return compiled;
}
