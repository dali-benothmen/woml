import {
  inspectCompiledWorkflowGraph,
  type CompiledWorkflowDefinition,
  type CompiledWorkflowMetadata,
  type CompiledWorkflowNode,
  type JsonValue,
} from './model';
import {
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
  readonly id: string;
  readonly source: string;
  readonly metadata?: Readonly<Record<string, JsonValue>>;
}

const supportedElements = new Set([
  'workflow',
  'triggers',
  'manual',
  'steps',
  'step',
  'script',
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
  'parallel',
  'branch',
  'when',
  'otherwise',
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
};

const workflowIdPattern = /^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$/;
const javascriptSafeIdPattern = /^[a-z][A-Za-z0-9]*$/;

function diagnostic(
  document: WomlSourceDocument,
  phase: 'validation' | 'compile',
  code: string,
  message: string,
  span: SourceSpan,
  hint?: string,
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
  hint?: string,
): never {
  throw new WomlValidationError(
    diagnostic(document, 'validation', code, message, span, hint),
  );
}

function failCompile(
  document: WomlSourceDocument,
  code: string,
  message: string,
  span: SourceSpan,
): never {
  throw new WomlCompileError(
    diagnostic(document, 'compile', code, message, span),
  );
}

function visitProfile(
  document: WomlSourceDocument,
  element: WomlSourceElement,
): void {
  if (!supportedElements.has(element.name)) {
    if (stagedElements.has(element.name)) {
      failValidation(
        document,
        'WOML_FEATURE_NOT_EXECUTABLE',
        `<${element.name}> is designed but is not executable in the first WOML CLI profile.`,
        element.openTagSpan,
      );
    }
    failValidation(
      document,
      'WOML_UNKNOWN_ELEMENT',
      `Unknown WOML element <${element.name}>.`,
      element.openTagSpan,
    );
  }

  const profile = elementProfiles[element.name];
  for (const attribute of Object.values(element.attributes)) {
    if (profile.stagedAttributes?.has(attribute.name) === true) {
      failValidation(
        document,
        'WOML_FEATURE_NOT_EXECUTABLE',
        `Attribute "${attribute.name}" on <${element.name}> is designed but is not executable in the first WOML CLI profile.`,
        attribute.nameSpan,
      );
    }
    if (!profile.attributes.has(attribute.name)) {
      failValidation(
        document,
        'WOML_UNKNOWN_ATTRIBUTE',
        `Unknown attribute "${attribute.name}" on <${element.name}>.`,
        attribute.nameSpan,
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
  name: string,
): WomlSourceAttribute {
  const attribute = element.attributes[name];
  if (attribute === undefined) {
    failValidation(
      document,
      'WOML_MISSING_ATTRIBUTE',
      `<${element.name}> requires the "${name}" attribute.`,
      element.openTagSpan,
    );
  }
  return attribute;
}

function elementChildren(
  document: WomlSourceDocument,
  parent: WomlSourceElement,
): WomlSourceElement[] {
  const elements: WomlSourceElement[] = [];
  for (const child of parent.children) {
    if (child.kind !== 'element') {
      failValidation(
        document,
        'WOML_UNEXPECTED_CONTENT',
        `<${parent.name}> may contain WOML elements only.`,
        child.span,
      );
    }
    elements.push(child);
  }
  return elements;
}

function ensureEmptyElement(
  document: WomlSourceDocument,
  element: WomlSourceElement,
): void {
  if (element.children.length > 0) {
    failValidation(
      document,
      'WOML_INVALID_STRUCTURE',
      `<${element.name}> cannot contain child content in the first WOML CLI profile.`,
      element.children[0].span,
    );
  }
}

function validateWorkflowId(
  document: WomlSourceDocument,
  attribute: WomlSourceAttribute,
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
      'Example: content-moderator',
    );
  }
  return attribute.value;
}

function validateJavaScriptSafeId(
  document: WomlSourceDocument,
  attribute: WomlSourceAttribute,
  role: 'trigger' | 'step',
): string {
  if (
    attribute.value.length > 256 ||
    !javascriptSafeIdPattern.test(attribute.value)
  ) {
    failValidation(
      document,
      'WOML_INVALID_ID',
      `${role === 'trigger' ? 'Trigger' : 'Step'} ID "${attribute.value}" must be a JavaScript-safe lower-camel identifier.`,
      attribute.valueSpan,
      'Use letters and numbers, start with a lowercase letter, and do not use hyphens.',
    );
  }
  return attribute.value;
}

function optionalMetadataValue(
  document: WomlSourceDocument,
  element: WomlSourceElement,
  name: 'name' | 'description' | 'version',
): string | undefined {
  const attribute = element.attributes[name];
  if (attribute === undefined) return undefined;
  if (attribute.value.trim().length === 0) {
    failValidation(
      document,
      'WOML_EMPTY_METADATA',
      `Attribute "${name}" on <${element.name}> must not be empty.`,
      attribute.valueSpan,
    );
  }
  return attribute.value;
}

function workflowMetadata(
  document: WomlSourceDocument,
  workflow: WomlSourceElement,
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

function stepMetadata(
  document: WomlSourceDocument,
  step: WomlSourceElement,
): Readonly<Record<string, JsonValue>> | undefined {
  const name = optionalMetadataValue(document, step, 'name');
  const description = optionalMetadataValue(document, step, 'description');
  if (name === undefined && description === undefined) return undefined;
  return {
    ...(name === undefined ? {} : { name }),
    ...(description === undefined ? {} : { description }),
  };
}

function validateWorkflowChildren(
  document: WomlSourceDocument,
  workflow: WomlSourceElement,
): readonly [WomlSourceElement, WomlSourceElement] {
  const children = elementChildren(document, workflow);
  const triggerContainers = children.filter((child) => child.name === 'triggers');
  const stepsContainers = children.filter((child) => child.name === 'steps');

  if (triggerContainers.length !== 1) {
    failValidation(
      document,
      'WOML_TRIGGER_CONTAINER_COUNT',
      `<workflow> requires exactly one <triggers> container; found ${triggerContainers.length}.`,
      triggerContainers[1]?.openTagSpan ?? workflow.openTagSpan,
    );
  }
  if (stepsContainers.length !== 1) {
    failValidation(
      document,
      'WOML_STEPS_CONTAINER_COUNT',
      `<workflow> requires exactly one <steps> container; found ${stepsContainers.length}.`,
      stepsContainers[1]?.openTagSpan ?? workflow.openTagSpan,
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
        index > 1,
    );
    failValidation(
      document,
      'WOML_INVALID_STRUCTURE',
      '<workflow> must contain <triggers> followed by <steps>, with no other executable-profile children.',
      offender?.openTagSpan ?? workflow.openTagSpan,
    );
  }

  return [triggerContainers[0], stepsContainers[0]];
}

function validateManualTrigger(
  document: WomlSourceDocument,
  triggers: WomlSourceElement,
): string {
  const children = elementChildren(document, triggers);
  if (children.length !== 1 || children[0].name !== 'manual') {
    const found =
      children.length === 0
        ? 'none'
        : children.map((child) => `<${child.name}>`).join(', ');
    failValidation(
      document,
      'WOML_MANUAL_TRIGGER_COUNT',
      `The first WOML CLI profile requires exactly one <manual> trigger; found ${found}.`,
      children[1]?.openTagSpan ?? children[0]?.openTagSpan ?? triggers.openTagSpan,
    );
  }

  const manual = children[0];
  ensureEmptyElement(document, manual);
  return validateJavaScriptSafeId(
    document,
    requiredAttribute(document, manual, 'id'),
    'trigger',
  );
}

function scriptSource(
  document: WomlSourceDocument,
  script: WomlSourceElement,
): string {
  const rawBodies = script.children.filter(
    (child): child is WomlSourceRawText => child.kind === 'raw',
  );
  if (
    script.children.some((child) => child.kind !== 'raw') ||
    rawBodies.length > 1
  ) {
    failValidation(
      document,
      'WOML_INVALID_STRUCTURE',
      '<script> must contain one raw JavaScript body and no WOML child elements.',
      script.children[0]?.span ?? script.openTagSpan,
    );
  }
  return rawBodies[0]?.value ?? '';
}

function validateSteps(
  document: WomlSourceDocument,
  stepsElement: WomlSourceElement,
): readonly ValidatedStep[] {
  const children = elementChildren(document, stepsElement);
  if (children.length === 0) {
    failValidation(
      document,
      'WOML_EMPTY_STEPS',
      '<steps> must contain at least one <step>.',
      stepsElement.openTagSpan,
    );
  }

  const ids = new Set<string>();
  return children.map((step) => {
    if (step.name !== 'step') {
      failValidation(
        document,
        'WOML_INVALID_STRUCTURE',
        `<steps> cannot contain <${step.name}> in the first WOML CLI profile.`,
        step.openTagSpan,
      );
    }

    const idAttribute = requiredAttribute(document, step, 'id');
    const id = validateJavaScriptSafeId(document, idAttribute, 'step');
    if (ids.has(id)) {
      failValidation(
        document,
        'WOML_DUPLICATE_ID',
        `Step ID "${id}" is duplicated.`,
        idAttribute.valueSpan,
      );
    }
    ids.add(id);

    const operations = elementChildren(document, step);
    if (operations.length !== 1 || operations[0].name !== 'script') {
      failValidation(
        document,
        'WOML_STEP_OPERATION_COUNT',
        `<step id="${id}"> must contain exactly one <script> operation.`,
        operations[1]?.openTagSpan ?? operations[0]?.openTagSpan ?? step.openTagSpan,
      );
    }

    return {
      id,
      source: scriptSource(document, operations[0]),
      metadata: stepMetadata(document, step),
    };
  });
}

function lowerNodes(steps: readonly ValidatedStep[]): readonly CompiledWorkflowNode[] {
  return steps.map((step) => ({
    id: step.id,
    handler: 'runtime.script',
    ...(step.metadata === undefined ? {} : { metadata: step.metadata }),
    inputs: {
      kind: 'object',
      fields: {
        source: { kind: 'literal', value: step.source },
      },
    },
  }));
}

export function compileWoml(
  document: WomlSourceDocument,
): CompiledWorkflowDefinition {
  const workflow = document.root;
  if (workflow.name !== 'workflow') {
    failValidation(
      document,
      'WOML_EXPECTED_WORKFLOW_ROOT',
      `Expected <workflow> as the document root, found <${workflow.name}>.`,
      workflow.openTagSpan,
    );
  }

  visitProfile(document, workflow);

  const workflowId = validateWorkflowId(
    document,
    requiredAttribute(document, workflow, 'id'),
  );
  const metadata = workflowMetadata(document, workflow);
  const [triggersElement, stepsElement] = validateWorkflowChildren(
    document,
    workflow,
  );
  const triggerId = validateManualTrigger(document, triggersElement);
  const steps = validateSteps(document, stepsElement);

  const compiled: CompiledWorkflowDefinition = {
    schemaVersion: 1,
    workflowId,
    ...(metadata === undefined ? {} : { metadata }),
    triggers: [
      {
        id: triggerId,
        handler: 'trigger.manual',
        config: { kind: 'object', fields: {} },
      },
    ],
    graph: {
      entryNodeIds: [steps[0].id],
      nodes: lowerNodes(steps),
      edges: steps.slice(1).map((step, index) => {
        const previous = steps[index];
        return {
          id: `${previous.id}-to-${step.id}`,
          from: previous.id,
          to: step.id,
          condition: { kind: 'always' as const },
        };
      }),
    },
  };

  const graphIssues = inspectCompiledWorkflowGraph(compiled.graph, {
    requireSingleTerminal: true,
  });
  if (graphIssues.length > 0) {
    failCompile(
      document,
      'WOML_INVALID_DAG',
      graphIssues[0].message,
      workflow.openTagSpan,
    );
  }

  return compiled;
}
