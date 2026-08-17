import { parse } from 'acorn';

import { isValidSecretName } from './secrets';

export interface ScriptAnalysisIssue {
  readonly code: string;
  readonly message: string;
  readonly start: number;
  readonly end: number;
  readonly hint?: string;
}

export interface ScriptServiceReference {
  readonly name: string;
  readonly start: number;
  readonly end: number;
}

export interface ScriptAnalysis {
  readonly requiredSecrets: readonly string[];
  readonly requiredServices: readonly string[];
  readonly serviceReferences: readonly ScriptServiceReference[];
  readonly usesServices: boolean;
  readonly usesNativeFetch: boolean;
  readonly issue?: ScriptAnalysisIssue;
}

interface AstNode {
  readonly type: string;
  readonly start: number;
  readonly end: number;
  readonly name?: string;
  readonly [key: string]: unknown;
}

interface AcornSyntaxError extends SyntaxError {
  readonly pos?: number;
  readonly raisedAt?: number;
}

const prefix =
  'async function __woml_script(context, lifecycle, attempt, services, secrets) {\n';
const suffix = '\n}';
const reservedBindings = new Set(['lifecycle', 'services', 'secrets', 'fetch']);
const unsupportedBunNetworkMethods = new Set([
  'connect',
  'listen',
  'serve',
  'udpSocket',
]);
const workflowIdPattern = /^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$/;
const operationNamePattern = /^[a-z][a-z0-9]*(?:[._-][a-z0-9]+)*$/;
const durationPattern = /^([1-9][0-9]*)(ms|s|m|h|d)$/;
const discordSnowflakePattern = /^[0-9]{17,20}$/;
const whatsAppPhoneNumberIdPattern = /^[1-9][0-9]{5,19}$/;
const whatsAppRecipientPattern = /^[1-9][0-9]{7,15}$/;
const whatsAppTemplatePattern = /^[a-z0-9_]+$/;
const whatsAppLanguagePattern = /^[a-z]{2,3}(?:_[A-Z]{2})?$/;
const durationMultipliers = {
  ms: 1,
  s: 1_000,
  m: 60_000,
  h: 3_600_000,
  d: 86_400_000,
} as const;
const maximumWorkflowCallTimeoutMs = 86_400_000;

function isNode(value: unknown): value is AstNode {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as { type?: unknown }).type === 'string' &&
    typeof (value as { start?: unknown }).start === 'number' &&
    typeof (value as { end?: unknown }).end === 'number'
  );
}

function sourceRange(node: AstNode, sourceLength: number) {
  const rawStart = node.start - prefix.length;
  const rawEnd = node.end - prefix.length;
  const start = Math.max(0, Math.min(rawStart, sourceLength));
  const end = Math.max(
    start + (start < sourceLength ? 1 : 0),
    Math.min(rawEnd, sourceLength)
  );
  return { start, end: Math.min(end, sourceLength) };
}

function issueAt(
  node: AstNode,
  sourceLength: number,
  code: string,
  message: string,
  hint?: string
): ScriptAnalysisIssue {
  return {
    code,
    message,
    ...sourceRange(node, sourceLength),
    ...(hint === undefined ? {} : { hint }),
  };
}

function bindingIdentifiers(node: unknown): readonly AstNode[] {
  if (!isNode(node)) return [];
  switch (node.type) {
    case 'Identifier':
      return [node];
    case 'ObjectPattern':
      return (
        (node.properties as readonly unknown[] | undefined) ?? []
      ).flatMap(property => {
        if (!isNode(property)) return [];
        if (property.type === 'RestElement') {
          return bindingIdentifiers(property.argument);
        }
        return bindingIdentifiers(property.value);
      });
    case 'ArrayPattern':
      return ((node.elements as readonly unknown[] | undefined) ?? []).flatMap(
        bindingIdentifiers
      );
    case 'AssignmentPattern':
      return bindingIdentifiers(node.left);
    case 'RestElement':
      return bindingIdentifiers(node.argument);
    default:
      return [];
  }
}

function declaredIdentifiers(node: AstNode): readonly AstNode[] {
  switch (node.type) {
    case 'VariableDeclarator':
      return bindingIdentifiers(node.id);
    case 'FunctionDeclaration':
    case 'FunctionExpression':
    case 'ArrowFunctionExpression':
      return [
        ...bindingIdentifiers(node.id),
        ...((node.params as readonly unknown[] | undefined) ?? []).flatMap(
          bindingIdentifiers
        ),
      ];
    case 'ClassDeclaration':
    case 'ClassExpression':
      return bindingIdentifiers(node.id);
    case 'CatchClause':
      return bindingIdentifiers(node.param);
    default:
      return [];
  }
}

function isDeclarationIdentifier(node: AstNode, parent?: AstNode): boolean {
  if (parent === undefined) return false;
  return declaredIdentifiers(parent).includes(node);
}

function isNonValueIdentifier(node: AstNode, parent?: AstNode): boolean {
  if (parent === undefined) return false;
  if (isDeclarationIdentifier(node, parent)) return true;
  if (
    parent.type === 'MemberExpression' &&
    parent.property === node &&
    parent.computed !== true
  ) {
    return true;
  }
  if (
    (parent.type === 'Property' || parent.type === 'PropertyDefinition') &&
    parent.key === node &&
    parent.computed !== true &&
    parent.shorthand !== true
  ) {
    return true;
  }
  if (
    (parent.type === 'MethodDefinition' ||
      parent.type === 'LabeledStatement') &&
    (parent.key === node || parent.label === node)
  ) {
    return true;
  }
  if (
    (parent.type === 'BreakStatement' || parent.type === 'ContinueStatement') &&
    parent.label === node
  ) {
    return true;
  }
  return false;
}

function children(node: AstNode): readonly AstNode[] {
  const result: AstNode[] = [];
  for (const [key, value] of Object.entries(node)) {
    if (key === 'start' || key === 'end' || key === 'type') continue;
    if (isNode(value)) {
      result.push(value);
    } else if (Array.isArray(value)) {
      for (const item of value) if (isNode(item)) result.push(item);
    }
  }
  return result;
}

function memberPropertyName(node: AstNode): string | undefined {
  if (node.type !== 'MemberExpression') return undefined;
  const property = node.property;
  if (!isNode(property)) return undefined;
  if (node.computed !== true && property.type === 'Identifier')
    return property.name;
  return node.computed === true &&
    property.type === 'Literal' &&
    typeof property.value === 'string'
    ? property.value
    : undefined;
}

function memberRootIdentifier(node: unknown): AstNode | undefined {
  let current = node;
  while (isNode(current) && current.type === 'MemberExpression') {
    current = current.object;
  }
  return isNode(current) && current.type === 'Identifier' ? current : undefined;
}

function staticMemberPath(node: unknown): readonly string[] | undefined {
  if (!isNode(node)) return undefined;
  if (node.type === 'Identifier' && node.name !== undefined) return [node.name];
  if (node.type !== 'MemberExpression' || node.computed === true)
    return undefined;
  const object = staticMemberPath(node.object);
  const property = node.property;
  if (
    object === undefined ||
    !isNode(property) ||
    property.type !== 'Identifier' ||
    property.name === undefined
  ) {
    return undefined;
  }
  return [...object, property.name];
}

function literalString(node: unknown): string | undefined {
  return isNode(node) &&
    node.type === 'Literal' &&
    typeof node.value === 'string'
    ? node.value
    : undefined;
}

function validLiteralWorkflowCallTimeout(node: AstNode): boolean {
  if (node.type !== 'Literal') return true;
  if (typeof node.value === 'number') {
    return (
      Number.isSafeInteger(node.value) &&
      node.value > 0 &&
      node.value <= maximumWorkflowCallTimeoutMs
    );
  }
  if (typeof node.value !== 'string') return false;
  const match = durationPattern.exec(node.value);
  if (match === null) return false;
  const amount = Number(match[1]);
  const milliseconds =
    amount * durationMultipliers[match[2] as keyof typeof durationMultipliers];
  return (
    Number.isSafeInteger(milliseconds) &&
    milliseconds > 0 &&
    milliseconds <= maximumWorkflowCallTimeoutMs
  );
}

function isWriteTarget(node: AstNode, parent?: AstNode): boolean {
  return (
    (parent?.type === 'AssignmentExpression' && parent.left === node) ||
    (parent?.type === 'UpdateExpression' && parent.argument === node) ||
    (parent?.type === 'UnaryExpression' &&
      parent.operator === 'delete' &&
      parent.argument === node)
  );
}

function parseIssue(
  error: AcornSyntaxError,
  sourceLength: number
): ScriptAnalysisIssue {
  const rawPosition = error.pos ?? error.raisedAt ?? prefix.length;
  const start = Math.max(
    0,
    Math.min(rawPosition - prefix.length, sourceLength)
  );
  return {
    code: 'WOML_SCRIPT_SYNTAX_INVALID',
    message: error.message.replace(/ \(\d+:\d+\)$/, ''),
    start,
    end: Math.min(Math.max(start + 1, start), sourceLength),
    hint: 'Fix the JavaScript syntax inside this <script> body.',
  };
}

function analyzeScript(
  source: string,
  mode: 'step' | 'lifecycle' | 'reusable-step' | 'notification-provider',
  options: { readonly shadowedServices?: readonly string[] } = {}
): ScriptAnalysis {
  let program: AstNode;
  try {
    program = parse(`${prefix}${source}${suffix}`, {
      ecmaVersion: 'latest',
      sourceType: 'script',
      allowAwaitOutsideFunction: false,
      allowReturnOutsideFunction: false,
    }) as unknown as AstNode;
  } catch (error) {
    return {
      requiredSecrets: [],
      requiredServices: [],
      serviceReferences: [],
      usesServices: false,
      usesNativeFetch: false,
      issue: parseIssue(error as AcornSyntaxError, source.length),
    };
  }

  const wrapperCandidate = (
    program.body as readonly unknown[] | undefined
  )?.find(node => isNode(node) && node.type === 'FunctionDeclaration');
  const wrapper = isNode(wrapperCandidate) ? wrapperCandidate : undefined;
  const body = isNode(wrapper?.body) ? wrapper.body : undefined;
  if (body === undefined) {
    return {
      requiredSecrets: [],
      requiredServices: [],
      serviceReferences: [],
      usesServices: false,
      usesNativeFetch: false,
      issue: {
        code: 'WOML_SCRIPT_SYNTAX_INVALID',
        message:
          'The script could not be parsed as an asynchronous function body.',
        start: 0,
        end: Math.min(source.length, 1),
      },
    };
  }

  const requiredSecrets = new Set<string>();
  const requiredServices = new Map<string, ScriptServiceReference>();
  let usesServices = false;
  let usesNativeFetch = false;
  let firstIssue: ScriptAnalysisIssue | undefined;

  const fail = (issue: ScriptAnalysisIssue) => {
    if (firstIssue === undefined || issue.start < firstIssue.start) {
      firstIssue = issue;
    }
  };

  const visit = (
    node: AstNode,
    parent?: AstNode,
    grandparent?: AstNode
  ): void => {
    const range = sourceRange(node, source.length);
    if (firstIssue !== undefined && range.start > firstIssue.start) return;

    if (node.type === 'ImportExpression') {
      fail(
        issueAt(
          node,
          source.length,
          'WOML_SCRIPT_DYNAMIC_IMPORT_UNSUPPORTED',
          'Dynamic import is unavailable in the current WOML script profile.',
          'Use the future WOML module system when it becomes available.'
        )
      );
    }

    if (node.type === 'CallExpression' && isNode(node.callee)) {
      const callee = node.callee;
      const path = staticMemberPath(callee);
      if (path?.[0] === 'services' && path[1] === 'workflows') {
        const workflowOperation = path[2];
        if (
          path.length !== 3 ||
          (workflowOperation !== 'call' && workflowOperation !== 'start')
        ) {
          fail(
            issueAt(
              callee,
              source.length,
              'WOML_WORKFLOW_OPERATION_UNSUPPORTED',
              `Workflow service operation "${path.slice(2).join('.') || ''}" is unsupported.`,
              'Use services.workflows.call() when the result is needed or services.workflows.start() to continue after durable admission.'
            )
          );
        } else {
          const args = (node.arguments as readonly unknown[] | undefined) ?? [];
          if (args.length < 2 || args.length > 3) {
            fail(
              issueAt(
                node,
                source.length,
                'WOML_WORKFLOW_CALL_ARGUMENTS_INVALID',
                `services.workflows.${workflowOperation}() requires workflowId, payload, and optional options.`,
                workflowOperation === 'call'
                  ? 'Use services.workflows.call("workflow-id", { ... }, { name, timeout }).'
                  : 'Use services.workflows.start("workflow-id", { ... }, { name }).'
              )
            );
          } else {
            const target = args[0];
            const targetLiteral = literalString(target);
            if (
              isNode(target) &&
              target.type === 'Literal' &&
              (targetLiteral === undefined ||
                targetLiteral.length > 256 ||
                !workflowIdPattern.test(targetLiteral))
            ) {
              fail(
                issueAt(
                  target,
                  source.length,
                  'WOML_WORKFLOW_TARGET_INVALID',
                  'A literal workflow target must use lowercase kebab-case.',
                  `Example: services.workflows.${workflowOperation}("calculate-risk", { ... }).`
                )
              );
            }

            const payload = args[1];
            if (
              isNode(payload) &&
              (payload.type === 'Literal' || payload.type === 'ArrayExpression')
            ) {
              fail(
                issueAt(
                  payload,
                  source.length,
                  'WOML_WORKFLOW_PAYLOAD_INVALID',
                  'A workflow call payload must be a top-level JSON object.',
                  'Pass an object literal or an expression that resolves to an object.'
                )
              );
            }

            const options = args[2];
            if (
              isNode(options) &&
              (options.type === 'Literal' || options.type === 'ArrayExpression')
            ) {
              fail(
                issueAt(
                  options,
                  source.length,
                  'WOML_WORKFLOW_CALL_OPTIONS_INVALID',
                  'Workflow call options must be an object.',
                  'Use { name: "logical-call", timeout: "30s" }.'
                )
              );
            } else if (isNode(options) && options.type === 'ObjectExpression') {
              for (const property of (options.properties as
                | readonly unknown[]
                | undefined) ?? []) {
                if (!isNode(property) || property.type !== 'Property') {
                  fail(
                    issueAt(
                      isNode(property) ? property : options,
                      source.length,
                      'WOML_WORKFLOW_CALL_OPTIONS_INVALID',
                      'Workflow call options do not support spread or computed members.',
                      'Use only static name and timeout properties.'
                    )
                  );
                  continue;
                }
                const name = isNode(property.key)
                  ? property.key.type === 'Identifier'
                    ? property.key.name
                    : literalString(property.key)
                  : undefined;
                if (
                  name !== 'name' &&
                  (workflowOperation !== 'call' || name !== 'timeout')
                ) {
                  fail(
                    issueAt(
                      property,
                      source.length,
                      'WOML_WORKFLOW_CALL_OPTION_UNKNOWN',
                      `Unknown workflow call option "${name ?? ''}".`,
                      workflowOperation === 'call'
                        ? 'Workflow Call v1 options are name and timeout.'
                        : 'Workflow Start v1 accepts only name.'
                    )
                  );
                }
                if (name === 'name') {
                  const value = literalString(property.value);
                  if (
                    isNode(property.value) &&
                    property.value.type === 'Literal' &&
                    (value === undefined ||
                      value.length > 128 ||
                      !operationNamePattern.test(value))
                  ) {
                    fail(
                      issueAt(
                        property.value,
                        source.length,
                        'WOML_WORKFLOW_CALL_NAME_INVALID',
                        'A literal workflow call name must use lowercase operation-name syntax.',
                        'Example: primary-risk'
                      )
                    );
                  }
                }
                if (
                  workflowOperation === 'call' &&
                  name === 'timeout' &&
                  isNode(property.value) &&
                  !validLiteralWorkflowCallTimeout(property.value)
                ) {
                  fail(
                    issueAt(
                      property.value,
                      source.length,
                      'WOML_WORKFLOW_CALL_OPTIONS_INVALID',
                      'A literal workflow call timeout must be a positive duration no greater than 24h.',
                      'Use milliseconds or a whole duration such as "30s", "5m", or "24h".'
                    )
                  );
                }
              }
            }
          }
        }
      }
      if (
        path?.[0] === 'services' &&
        path[1] === 'telegram' &&
        !options.shadowedServices?.includes('telegram')
      ) {
        if (path.length !== 3 || path[2] !== 'send') {
          fail(
            issueAt(
              callee,
              source.length,
              'WOML_TELEGRAM_OPERATION_UNSUPPORTED',
              `Telegram service operation "${path.slice(2).join('.') || ''}" is unsupported.`,
              'Use services.telegram.send({ botToken, conversationId, text }).'
            )
          );
        } else {
          const args = (node.arguments as readonly unknown[] | undefined) ?? [];
          if (args.length < 1 || args.length > 2) {
            fail(
              issueAt(
                node,
                source.length,
                'WOML_TELEGRAM_SEND_ARGUMENTS_INVALID',
                'services.telegram.send() requires one request object and optional operation options.',
                'Use services.telegram.send({ botToken: secrets.TELEGRAM_BOT_TOKEN, conversationId, text }, { name: "reply" }).'
              )
            );
          } else {
            const input = args[0];
            if (!isNode(input) || input.type !== 'ObjectExpression') {
              fail(
                issueAt(
                  isNode(input) ? input : node,
                  source.length,
                  'WOML_TELEGRAM_SEND_INPUT_INVALID',
                  'A workflow script must pass a direct object to services.telegram.send().',
                  'Use the required botToken, conversationId, and text properties.'
                )
              );
            } else {
              const seen = new Set<string>();
              for (const property of (input.properties as readonly unknown[] | undefined) ?? []) {
                if (
                  !isNode(property) ||
                  property.type !== 'Property' ||
                  property.computed === true ||
                  property.kind !== 'init'
                ) {
                  fail(
                    issueAt(
                      isNode(property) ? property : input,
                      source.length,
                      'WOML_TELEGRAM_SEND_INPUT_INVALID',
                      'Telegram send input does not support spreads, computed keys, getters, or setters.'
                    )
                  );
                  continue;
                }
                const name = isNode(property.key)
                  ? property.key.type === 'Identifier'
                    ? property.key.name
                    : literalString(property.key)
                  : undefined;
                if (
                  name !== 'botToken' &&
                  name !== 'conversationId' &&
                  name !== 'text' &&
                  name !== 'replyToMessageId'
                ) {
                  fail(
                    issueAt(
                      property,
                      source.length,
                      'WOML_TELEGRAM_SEND_PROPERTY_UNKNOWN',
                      `Unknown Telegram send property "${name ?? ''}".`,
                      'Telegram Send v1 accepts botToken, conversationId, text, and replyToMessageId.'
                    )
                  );
                  continue;
                }
                if (seen.has(name)) {
                  fail(
                    issueAt(
                      property,
                      source.length,
                      'WOML_TELEGRAM_SEND_PROPERTY_DUPLICATE',
                      `Telegram send property "${name}" is declared more than once.`
                    )
                  );
                }
                seen.add(name);
                if (name === 'botToken') {
                  const credentialPath = isNode(property.value)
                    ? staticMemberPath(property.value)
                    : undefined;
                  if (
                    credentialPath?.length !== 2 ||
                    credentialPath[0] !== 'secrets' ||
                    !isValidSecretName(credentialPath[1])
                  ) {
                    fail(
                      issueAt(
                        isNode(property.value) ? property.value : property,
                        source.length,
                        'WOML_TELEGRAM_CREDENTIAL_INVALID',
                        'Telegram botToken must be one direct secrets.NAME value.',
                        'Example: botToken: secrets.TELEGRAM_BOT_TOKEN'
                      )
                    );
                  }
                }
                const literal = literalString(property.value);
                if (
                  literal !== undefined &&
                  (literal.length === 0 ||
                    literal.length > (name === 'text' ? 40_000 : 320))
                ) {
                  fail(
                    issueAt(
                      property.value as AstNode,
                      source.length,
                      'WOML_TELEGRAM_SEND_VALUE_INVALID',
                      `Telegram send property "${name}" has an invalid literal value.`
                    )
                  );
                }
              }
              for (const required of ['botToken', 'conversationId', 'text']) {
                if (!seen.has(required)) {
                  fail(
                    issueAt(
                      input,
                      source.length,
                      'WOML_TELEGRAM_SEND_PROPERTY_REQUIRED',
                      `services.telegram.send() requires the "${required}" property.`
                    )
                  );
                }
              }
            }
            const options = args[1];
            if (isNode(options)) {
              if (options.type !== 'ObjectExpression') {
                fail(
                  issueAt(
                    options,
                    source.length,
                    'WOML_TELEGRAM_SEND_OPTIONS_INVALID',
                    'Telegram send operation options must be an object containing an optional stable name.'
                  )
                );
              } else {
                for (const property of (options.properties as readonly unknown[] | undefined) ?? []) {
                  const name =
                    isNode(property) && property.type === 'Property' && isNode(property.key)
                      ? property.key.type === 'Identifier'
                        ? property.key.name
                        : literalString(property.key)
                      : undefined;
                  if (name !== 'name') {
                    fail(
                      issueAt(
                        isNode(property) ? property : options,
                        source.length,
                        'WOML_TELEGRAM_SEND_OPTION_UNKNOWN',
                        `Unknown Telegram send option "${name ?? ''}".`,
                        'Telegram Send v1 accepts only the stable name option.'
                      )
                    );
                  }
                }
              }
            }
          }
        }
      }
      if (
        path?.[0] === 'services' &&
        path[1] === 'discord' &&
        !options.shadowedServices?.includes('discord')
      ) {
        if (path.length !== 3 || path[2] !== 'send') {
          fail(
            issueAt(
              callee,
              source.length,
              'WOML_DISCORD_OPERATION_UNSUPPORTED',
              `Discord service operation "${path.slice(2).join('.') || ''}" is unsupported.`,
              'Use services.discord.send({ botToken, conversationId, text }).'
            )
          );
        } else {
          const args = (node.arguments as readonly unknown[] | undefined) ?? [];
          if (args.length < 1 || args.length > 2) {
            fail(
              issueAt(
                node,
                source.length,
                'WOML_DISCORD_SEND_ARGUMENTS_INVALID',
                'services.discord.send() requires one request object and optional operation options.',
                'Use services.discord.send({ botToken: secrets.DISCORD_BOT_TOKEN, conversationId, text }, { name: "reply" }).'
              )
            );
          } else {
            const input = args[0];
            if (!isNode(input) || input.type !== 'ObjectExpression') {
              fail(
                issueAt(
                  isNode(input) ? input : node,
                  source.length,
                  'WOML_DISCORD_SEND_INPUT_INVALID',
                  'A workflow script must pass a direct object to services.discord.send().',
                  'Use the required botToken, conversationId, and text properties.'
                )
              );
            } else {
              const seen = new Set<string>();
              for (const property of (input.properties as readonly unknown[] | undefined) ?? []) {
                if (
                  !isNode(property) ||
                  property.type !== 'Property' ||
                  property.computed === true ||
                  property.kind !== 'init'
                ) {
                  fail(
                    issueAt(
                      isNode(property) ? property : input,
                      source.length,
                      'WOML_DISCORD_SEND_INPUT_INVALID',
                      'Discord send input does not support spreads, computed keys, getters, or setters.'
                    )
                  );
                  continue;
                }
                const name = isNode(property.key)
                  ? property.key.type === 'Identifier'
                    ? property.key.name
                    : literalString(property.key)
                  : undefined;
                if (
                  name !== 'botToken' &&
                  name !== 'conversationId' &&
                  name !== 'text' &&
                  name !== 'replyToMessageId'
                ) {
                  fail(
                    issueAt(
                      property,
                      source.length,
                      'WOML_DISCORD_SEND_PROPERTY_UNKNOWN',
                      `Unknown Discord send property "${name ?? ''}".`,
                      'Discord Send v1 accepts botToken, conversationId, text, and replyToMessageId.'
                    )
                  );
                  continue;
                }
                if (seen.has(name)) {
                  fail(
                    issueAt(
                      property,
                      source.length,
                      'WOML_DISCORD_SEND_PROPERTY_DUPLICATE',
                      `Discord send property "${name}" is declared more than once.`
                    )
                  );
                }
                seen.add(name);
                if (name === 'botToken') {
                  const credentialPath = isNode(property.value)
                    ? staticMemberPath(property.value)
                    : undefined;
                  if (
                    credentialPath?.length !== 2 ||
                    credentialPath[0] !== 'secrets' ||
                    !isValidSecretName(credentialPath[1])
                  ) {
                    fail(
                      issueAt(
                        isNode(property.value) ? property.value : property,
                        source.length,
                        'WOML_DISCORD_CREDENTIAL_INVALID',
                        'Discord botToken must be one direct secrets.NAME value.',
                        'Example: botToken: secrets.DISCORD_BOT_TOKEN'
                      )
                    );
                  }
                }
                const literal = literalString(property.value);
                if (
                  literal !== undefined &&
                  (literal.length === 0 ||
                    literal.length > (name === 'text' ? 2_000 : 320))
                ) {
                  fail(
                    issueAt(
                      property.value as AstNode,
                      source.length,
                      'WOML_DISCORD_SEND_VALUE_INVALID',
                      `Discord send property "${name}" has an invalid literal value.`
                    )
                  );
                }
                if (
                  literal !== undefined &&
                  (name === 'conversationId' ||
                    name === 'replyToMessageId') &&
                  !discordSnowflakePattern.test(literal)
                ) {
                  fail(
                    issueAt(
                      property.value as AstNode,
                      source.length,
                      'WOML_DISCORD_SEND_VALUE_INVALID',
                      `Discord send property "${name}" must be a numeric Discord snowflake containing 17 to 20 digits when written as a literal.`
                    )
                  );
                }
              }
              for (const required of ['botToken', 'conversationId', 'text']) {
                if (!seen.has(required)) {
                  fail(
                    issueAt(
                      input,
                      source.length,
                      'WOML_DISCORD_SEND_PROPERTY_REQUIRED',
                      `services.discord.send() requires the "${required}" property.`
                    )
                  );
                }
              }
            }
            const operationOptions = args[1];
            if (isNode(operationOptions)) {
              if (operationOptions.type !== 'ObjectExpression') {
                fail(
                  issueAt(
                    operationOptions,
                    source.length,
                    'WOML_DISCORD_SEND_OPTIONS_INVALID',
                    'Discord send operation options must be an object containing an optional stable name.'
                  )
                );
              } else {
                const seenOptions = new Set<string>();
                for (const property of (operationOptions.properties as readonly unknown[] | undefined) ?? []) {
                  if (
                    !isNode(property) ||
                    property.type !== 'Property' ||
                    property.computed === true ||
                    property.kind !== 'init'
                  ) {
                    fail(
                      issueAt(
                        isNode(property) ? property : operationOptions,
                        source.length,
                        'WOML_DISCORD_SEND_OPTIONS_INVALID',
                        'Discord send options do not support spreads, computed keys, getters, or setters.'
                      )
                    );
                    continue;
                  }
                  const name = isNode(property.key)
                    ? property.key.type === 'Identifier'
                      ? property.key.name
                      : literalString(property.key)
                    : undefined;
                  if (name !== 'name') {
                    fail(
                      issueAt(
                        property,
                        source.length,
                        'WOML_DISCORD_SEND_OPTION_UNKNOWN',
                        `Unknown Discord send option "${name ?? ''}".`,
                        'Discord Send v1 accepts only the stable name option.'
                      )
                    );
                    continue;
                  }
                  if (seenOptions.has(name)) {
                    fail(
                      issueAt(
                        property,
                        source.length,
                        'WOML_DISCORD_SEND_OPTION_DUPLICATE',
                        'Discord send option "name" is declared more than once.'
                      )
                    );
                  }
                  seenOptions.add(name);
                  const value = literalString(property.value);
                  if (
                    isNode(property.value) &&
                    property.value.type === 'Literal' &&
                    (value === undefined ||
                      value.length > 128 ||
                      !operationNamePattern.test(value))
                  ) {
                    fail(
                      issueAt(
                        property.value,
                        source.length,
                        'WOML_DISCORD_SEND_NAME_INVALID',
                        'A literal Discord send name must use lowercase operation-name syntax.',
                        'Example: reply-to-message'
                      )
                    );
                  }
                }
              }
            }
          }
        }
      }
      if (
        path?.[0] === 'services' &&
        path[1] === 'whatsapp' &&
        !options.shadowedServices?.includes('whatsapp')
      ) {
        if (path.length !== 3 || path[2] !== 'send') {
          fail(
            issueAt(
              callee,
              source.length,
              'WOML_WHATSAPP_OPERATION_UNSUPPORTED',
              `WhatsApp service operation "${path.slice(2).join('.') || ''}" is unsupported.`,
              'Use services.whatsapp.send({ accessToken, phoneNumberId, conversationId, template }).'
            )
          );
        } else {
          const args = (node.arguments as readonly unknown[] | undefined) ?? [];
          const input = args[0];
          if (args.length < 1 || args.length > 2) {
            fail(
              issueAt(
                node,
                source.length,
                'WOML_WHATSAPP_SEND_ARGUMENTS_INVALID',
                'services.whatsapp.send() requires one request object and optional operation options.'
              )
            );
          } else if (!isNode(input) || input.type !== 'ObjectExpression') {
            fail(
              issueAt(
                isNode(input) ? input : node,
                source.length,
                'WOML_WHATSAPP_SEND_INPUT_INVALID',
                'A workflow script must pass a direct object to services.whatsapp.send().'
              )
            );
          } else {
            const seen = new Set<string>();
            for (const property of (input.properties as readonly unknown[] | undefined) ?? []) {
              if (
                !isNode(property) ||
                property.type !== 'Property' ||
                property.computed === true ||
                property.kind !== 'init'
              ) {
                fail(
                  issueAt(
                    isNode(property) ? property : input,
                    source.length,
                    'WOML_WHATSAPP_SEND_INPUT_INVALID',
                    'WhatsApp send input does not support spreads, computed keys, getters, or setters.'
                  )
                );
                continue;
              }
              const name = isNode(property.key)
                ? property.key.type === 'Identifier'
                  ? property.key.name
                  : literalString(property.key)
                : undefined;
              if (
                name !== 'accessToken' &&
                name !== 'phoneNumberId' &&
                name !== 'conversationId' &&
                name !== 'template'
              ) {
                fail(
                  issueAt(
                    property,
                    source.length,
                    'WOML_WHATSAPP_SEND_PROPERTY_UNKNOWN',
                    `Unknown WhatsApp send property "${name ?? ''}".`,
                    'WhatsApp Send v1 accepts accessToken, phoneNumberId, conversationId, and template.'
                  )
                );
                continue;
              }
              if (seen.has(name)) {
                fail(
                  issueAt(
                    property,
                    source.length,
                    'WOML_WHATSAPP_SEND_PROPERTY_DUPLICATE',
                    `WhatsApp send property "${name}" is declared more than once.`
                  )
                );
              }
              seen.add(name);
              if (name === 'accessToken') {
                const credentialPath = isNode(property.value)
                  ? staticMemberPath(property.value)
                  : undefined;
                if (
                  credentialPath?.length !== 2 ||
                  credentialPath[0] !== 'secrets' ||
                  !isValidSecretName(credentialPath[1])
                ) {
                  fail(
                    issueAt(
                      isNode(property.value) ? property.value : property,
                      source.length,
                      'WOML_WHATSAPP_CREDENTIAL_INVALID',
                      'WhatsApp accessToken must be one direct secrets.NAME value.',
                      'Example: accessToken: secrets.WHATSAPP_ACCESS_TOKEN'
                    )
                  );
                }
              }
              const literal = literalString(property.value);
              if (
                literal !== undefined &&
                name === 'phoneNumberId' &&
                !whatsAppPhoneNumberIdPattern.test(literal)
              ) {
                fail(issueAt(property.value as AstNode, source.length, 'WOML_WHATSAPP_SEND_VALUE_INVALID', 'WhatsApp phoneNumberId must be the numeric Meta Phone Number ID.'));
              }
              if (
                literal !== undefined &&
                name === 'conversationId' &&
                !whatsAppRecipientPattern.test(literal)
              ) {
                fail(issueAt(property.value as AstNode, source.length, 'WOML_WHATSAPP_SEND_VALUE_INVALID', 'WhatsApp conversationId must be an international phone number containing 8 to 16 digits without a plus sign.'));
              }
              if (name === 'template') {
                if (!isNode(property.value) || property.value.type !== 'ObjectExpression') {
                  fail(issueAt(isNode(property.value) ? property.value : property, source.length, 'WOML_WHATSAPP_TEMPLATE_INVALID', 'WhatsApp template must be a direct object with name, language, and parameters.'));
                  continue;
                }
                const templateSeen = new Set<string>();
                for (const templateProperty of (property.value.properties as readonly unknown[] | undefined) ?? []) {
                  if (!isNode(templateProperty) || templateProperty.type !== 'Property' || templateProperty.computed === true || templateProperty.kind !== 'init') {
                    fail(issueAt(isNode(templateProperty) ? templateProperty : property.value, source.length, 'WOML_WHATSAPP_TEMPLATE_INVALID', 'WhatsApp template does not support spreads, computed keys, getters, or setters.'));
                    continue;
                  }
                  const templateKey = isNode(templateProperty.key)
                    ? templateProperty.key.type === 'Identifier'
                      ? templateProperty.key.name
                      : literalString(templateProperty.key)
                    : undefined;
                  if (templateKey !== 'name' && templateKey !== 'language' && templateKey !== 'parameters') {
                    fail(issueAt(templateProperty, source.length, 'WOML_WHATSAPP_TEMPLATE_PROPERTY_UNKNOWN', `Unknown WhatsApp template property "${templateKey ?? ''}".`));
                    continue;
                  }
                  if (templateSeen.has(templateKey)) {
                    fail(issueAt(templateProperty, source.length, 'WOML_WHATSAPP_TEMPLATE_PROPERTY_DUPLICATE', `WhatsApp template property "${templateKey}" is declared more than once.`));
                  }
                  templateSeen.add(templateKey);
                  const templateLiteral = literalString(templateProperty.value);
                  if (templateKey === 'name' && (templateLiteral === undefined || !whatsAppTemplatePattern.test(templateLiteral))) {
                    fail(issueAt(templateProperty.value as AstNode, source.length, 'WOML_WHATSAPP_TEMPLATE_INVALID', 'WhatsApp template name must be a literal containing lowercase letters, digits, and underscores only.'));
                  }
                  if (templateKey === 'language' && (templateLiteral === undefined || !whatsAppLanguagePattern.test(templateLiteral))) {
                    fail(issueAt(templateProperty.value as AstNode, source.length, 'WOML_WHATSAPP_LANGUAGE_INVALID', 'WhatsApp template language must be a literal such as en, en_US, or pt_BR.'));
                  }
                  if (templateKey === 'parameters' && isNode(templateProperty.value)) {
                    if (templateProperty.value.type !== 'ArrayExpression') {
                      fail(issueAt(templateProperty.value, source.length, 'WOML_WHATSAPP_TEMPLATE_PARAMETERS_INVALID', 'WhatsApp template parameters must be a direct array.'));
                    } else {
                      const elements = (templateProperty.value.elements as readonly unknown[] | undefined) ?? [];
                      if (elements.length > 32 || elements.some(value => isNode(value) && (literalString(value)?.length ?? 0) > 1024)) {
                        fail(issueAt(templateProperty.value, source.length, 'WOML_WHATSAPP_TEMPLATE_PARAMETERS_INVALID', 'WhatsApp templates accept at most 32 parameters of at most 1024 characters each.'));
                      }
                    }
                  }
                }
                for (const required of ['name', 'language', 'parameters']) {
                  if (!templateSeen.has(required)) {
                    fail(issueAt(property.value, source.length, 'WOML_WHATSAPP_TEMPLATE_PROPERTY_REQUIRED', `WhatsApp template requires the "${required}" property.`));
                  }
                }
              }
            }
            for (const required of ['accessToken', 'phoneNumberId', 'conversationId', 'template']) {
              if (!seen.has(required)) {
                fail(issueAt(input, source.length, 'WOML_WHATSAPP_SEND_PROPERTY_REQUIRED', `services.whatsapp.send() requires the "${required}" property.`));
              }
            }
            const operationOptions = args[1];
            if (isNode(operationOptions)) {
              if (operationOptions.type !== 'ObjectExpression') {
                fail(issueAt(operationOptions, source.length, 'WOML_WHATSAPP_SEND_OPTIONS_INVALID', 'WhatsApp send operation options must be an object containing an optional stable name.'));
              } else {
                const seenOptions = new Set<string>();
                for (const property of (operationOptions.properties as readonly unknown[] | undefined) ?? []) {
                  if (!isNode(property) || property.type !== 'Property' || property.computed === true || property.kind !== 'init') {
                    fail(issueAt(isNode(property) ? property : operationOptions, source.length, 'WOML_WHATSAPP_SEND_OPTIONS_INVALID', 'WhatsApp send options do not support spreads, computed keys, getters, or setters.'));
                    continue;
                  }
                  const name = isNode(property.key)
                    ? property.key.type === 'Identifier'
                      ? property.key.name
                      : literalString(property.key)
                    : undefined;
                  if (name !== 'name') {
                    fail(issueAt(property, source.length, 'WOML_WHATSAPP_SEND_OPTION_UNKNOWN', `Unknown WhatsApp send option "${name ?? ''}".`, 'WhatsApp Send v1 accepts only the stable name option.'));
                    continue;
                  }
                  if (seenOptions.has(name)) {
                    fail(issueAt(property, source.length, 'WOML_WHATSAPP_SEND_OPTION_DUPLICATE', 'WhatsApp send option "name" is declared more than once.'));
                  }
                  seenOptions.add(name);
                  const value = literalString(property.value);
                  if (isNode(property.value) && property.value.type === 'Literal' && (value === undefined || value.length > 128 || !operationNamePattern.test(value))) {
                    fail(issueAt(property.value, source.length, 'WOML_WHATSAPP_SEND_NAME_INVALID', 'A literal WhatsApp send name must use lowercase operation-name syntax.', 'Example: send-template'));
                  }
                }
              }
            }
          }
        }
      }
      if (callee.type === 'Identifier' && callee.name === 'require') {
        fail(
          issueAt(
            node,
            source.length,
            'WOML_SCRIPT_MODULE_REQUIRE_UNSUPPORTED',
            'Runtime module loading is unavailable in the current WOML script profile.',
            'Use the future WOML module system when it becomes available.'
          )
        );
      }
      if (
        callee.type === 'MemberExpression' &&
        memberRootIdentifier(callee)?.name === 'Bun' &&
        unsupportedBunNetworkMethods.has(memberPropertyName(callee) ?? '')
      ) {
        fail(
          issueAt(
            node,
            source.length,
            'WOML_SCRIPT_RAW_NETWORK_UNSUPPORTED',
            'Raw Bun networking is unavailable in the tracked WOML script profile.',
            'Use native fetch() or a managed services operation.'
          )
        );
      }
    }

    if (
      (node.type === 'CallExpression' || node.type === 'NewExpression') &&
      isNode(node.callee) &&
      node.callee.type === 'Identifier' &&
      node.callee.name === 'WebSocket'
    ) {
      fail(
        issueAt(
          node,
          source.length,
          'WOML_SCRIPT_RAW_NETWORK_UNSUPPORTED',
          'WebSocket networking is unavailable in the tracked WOML script profile.',
          'Use native fetch() or a managed services operation.'
        )
      );
    }

    for (const identifier of declaredIdentifiers(node)) {
      if (
        identifier.name !== undefined &&
        (reservedBindings.has(identifier.name) ||
          ((mode === 'reusable-step' || mode === 'notification-provider') &&
            (identifier.name === 'props' ||
              (mode === 'notification-provider' &&
                identifier.name === 'notification'))))
      ) {
        fail(
          issueAt(
            identifier,
            source.length,
            'WOML_SCRIPT_BINDING_SHADOWED',
            `The reserved WOML binding "${identifier.name}" cannot be redeclared or shadowed.`,
            `Rename this local binding; WOML provides ${identifier.name} at runtime.`
          )
        );
      }
    }

    if (
      node.type === 'MemberExpression' &&
      memberRootIdentifier(node)?.name === 'services'
    ) {
      if (node.computed === true) {
        fail(
          issueAt(
            node,
            source.length,
            'WOML_SCRIPT_SERVICE_ACCESS_DYNAMIC',
            'Computed service access is not executable in Script Bindings v1.',
            'Use static service and operation names such as services.http.request.'
          )
        );
      } else if (node.optional === true) {
        fail(
          issueAt(
            node,
            source.length,
            'WOML_SCRIPT_SERVICE_ACCESS_UNSUPPORTED',
            'Optional access is not supported for the services binding.',
            'Use a static service operation such as services.http.request.'
          )
        );
      } else if (isWriteTarget(node, parent)) {
        fail(
          issueAt(
            node,
            source.length,
            'WOML_SCRIPT_SERVICE_WRITE_UNSUPPORTED',
            'The services binding is read-only.',
            'Call a service operation; do not replace or delete service members.'
          )
        );
      }
    }

    if (
      node.type === 'MemberExpression' &&
      memberRootIdentifier(node)?.name === 'lifecycle' &&
      isWriteTarget(node, parent)
    ) {
      fail(
        issueAt(
          node,
          source.length,
          'WOML_LIFECYCLE_BINDING_READ_ONLY',
          'The lifecycle binding is read-only.',
          'Read lifecycle event data without replacing or deleting it.'
        )
      );
    }

    if (
      (mode === 'reusable-step' || mode === 'notification-provider') &&
      node.type === 'MemberExpression' &&
      (memberRootIdentifier(node)?.name === 'props' ||
        (mode === 'notification-provider' &&
          memberRootIdentifier(node)?.name === 'notification')) &&
      isWriteTarget(node, parent)
    ) {
      fail(
        issueAt(
          node,
          source.length,
          memberRootIdentifier(node)?.name === 'props'
            ? 'WOML_REUSABLE_PROPS_READ_ONLY'
            : 'WOML_PROVIDER_NOTIFICATION_READ_ONLY',
          memberRootIdentifier(node)?.name === 'props'
            ? 'The reusable props binding is immutable.'
            : 'The provider notification binding is immutable.',
          'Read provider bindings without replacing, deleting, or updating them.'
        )
      );
    }

    if (
      node.type === 'Identifier' &&
      node.name !== undefined &&
      !isNonValueIdentifier(node, parent)
    ) {
      if (node.name === 'lifecycle') {
        if (mode !== 'lifecycle') {
          fail(
            issueAt(
              node,
              source.length,
              'WOML_LIFECYCLE_BINDING_UNAVAILABLE',
              'The lifecycle binding is available only inside <lifecycle> scripts.',
              'Use context inside a normal workflow step, or move this observer into <lifecycle>.'
            )
          );
        } else if (isWriteTarget(node, parent)) {
          fail(
            issueAt(
              node,
              source.length,
              'WOML_LIFECYCLE_BINDING_READ_ONLY',
              'The lifecycle binding is read-only.'
            )
          );
        }
      } else if (
        node.name === 'props' &&
        (mode === 'reusable-step' || mode === 'notification-provider')
      ) {
        if (isWriteTarget(node, parent)) {
          fail(
            issueAt(
              node,
              source.length,
              'WOML_REUSABLE_PROPS_READ_ONLY',
              'The reusable props binding is immutable.'
            )
          );
        }
      } else if (
        node.name === 'notification' &&
        mode === 'notification-provider'
      ) {
        if (isWriteTarget(node, parent)) {
          fail(
            issueAt(
              node,
              source.length,
              'WOML_PROVIDER_NOTIFICATION_READ_ONLY',
              'The provider notification binding is immutable.'
            )
          );
        }
      } else if (node.name === 'context' && mode === 'notification-provider') {
        fail(
          issueAt(
            node,
            source.length,
            'WOML_PROVIDER_CONTEXT_UNAVAILABLE',
            'Notification provider scripts cannot access workflow context directly.',
            'Pass required business values through declared props.'
          )
        );
      } else if (node.name === 'secrets') {
        if (mode === 'reusable-step' || mode === 'notification-provider') {
          fail(
            issueAt(
              node,
              source.length,
              'WOML_REUSABLE_SECRET_ACCESS_FORBIDDEN',
              'Reusable definition scripts cannot access secrets directly.',
              'Declare a secret prop and pass one exact {{secrets.NAME}} reference at the invocation.'
            )
          );
        }
        if (parent?.type === 'MemberExpression' && parent.object === node) {
          if (isWriteTarget(parent, grandparent)) {
            fail(
              issueAt(
                parent,
                source.length,
                'WOML_SCRIPT_SECRET_WRITE_UNSUPPORTED',
                'The secrets binding is read-only.',
                'Read the required value with secrets.SECRET_NAME; do not replace or delete it.'
              )
            );
          } else if (parent.optional === true) {
            fail(
              issueAt(
                parent,
                source.length,
                'WOML_SCRIPT_SECRET_ACCESS_UNSUPPORTED',
                'Optional access is not supported for the required secrets binding.',
                'Use the exact form secrets.SECRET_NAME.'
              )
            );
          } else if (parent.computed === true) {
            fail(
              issueAt(
                parent,
                source.length,
                'WOML_SCRIPT_SECRET_ACCESS_DYNAMIC',
                'Computed secret access is not executable in Script Bindings v1.',
                'Use the exact literal property form secrets.SECRET_NAME.'
              )
            );
          } else {
            const name = memberPropertyName(parent);
            if (name === undefined || !isValidSecretName(name)) {
              fail(
                issueAt(
                  isNode(parent.property) ? parent.property : parent,
                  source.length,
                  'WOML_SCRIPT_SECRET_NAME_INVALID',
                  `Invalid script secret name "${name ?? ''}".`,
                  'Secret names must start with A-Z and contain only A-Z, 0-9, or underscore.'
                )
              );
            } else {
              requiredSecrets.add(name);
            }
          }
        } else {
          fail(
            issueAt(
              node,
              source.length,
              'WOML_SCRIPT_SECRET_ACCESS_UNSUPPORTED',
              'The secrets binding cannot be enumerated, aliased, returned, or passed as a whole object.',
              'Access only the exact value needed with secrets.SECRET_NAME.'
            )
          );
        }
      } else if (node.name === 'services') {
        if (parent?.type === 'MemberExpression' && parent.object === node) {
          if (parent.optional === true) {
            fail(
              issueAt(
                parent,
                source.length,
                'WOML_SCRIPT_SERVICE_ACCESS_UNSUPPORTED',
                'Optional access is not supported for the services binding.',
                'Use a static service name such as services.http.'
              )
            );
          } else if (parent.computed === true) {
            fail(
              issueAt(
                parent,
                source.length,
                'WOML_SCRIPT_SERVICE_ACCESS_DYNAMIC',
                'Computed service access is not executable in Script Bindings v1.',
                'Use a static service name such as services.http.'
              )
            );
          } else {
            usesServices = true;
            const name = memberPropertyName(parent);
            if (name !== undefined && !requiredServices.has(name)) {
              requiredServices.set(name, {
                name,
                ...sourceRange(parent, source.length),
              });
            }
          }
        } else {
          fail(
            issueAt(
              node,
              source.length,
              'WOML_SCRIPT_SERVICE_ACCESS_UNSUPPORTED',
              'The services binding cannot be enumerated, aliased, returned, or passed as a whole object.',
              'Call a static service namespace such as services.http.'
            )
          );
        }
      } else if (node.name === 'fetch') {
        if (isWriteTarget(node, parent)) {
          fail(
            issueAt(
              node,
              source.length,
              'WOML_SCRIPT_FETCH_WRITE_UNSUPPORTED',
              'The tracked native fetch binding cannot be replaced or deleted.',
              'Call fetch() directly without modifying the runtime binding.'
            )
          );
        } else {
          usesNativeFetch = true;
        }
      } else if (
        (node.name === 'globalThis' || node.name === 'self') &&
        parent?.type === 'MemberExpression' &&
        parent.object === node &&
        memberPropertyName(parent) === 'fetch'
      ) {
        if (isWriteTarget(parent, grandparent)) {
          fail(
            issueAt(
              parent,
              source.length,
              'WOML_SCRIPT_FETCH_WRITE_UNSUPPORTED',
              'The tracked native fetch binding cannot be replaced or deleted.',
              'Call fetch() directly without modifying the runtime binding.'
            )
          );
        } else {
          usesNativeFetch = true;
        }
      }
    }

    for (const child of children(node)) visit(child, node, parent);
  };

  for (const statement of (body.body as readonly unknown[] | undefined) ?? []) {
    if (isNode(statement)) visit(statement);
  }

  const names = [...requiredSecrets].sort();
  if (names.length > 64) {
    firstIssue = issueAt(
      body,
      source.length,
      'WOML_SCRIPT_SECRET_LIMIT_EXCEEDED',
      'One script may reference at most 64 distinct secrets.',
      'Split the operation into smaller steps with explicit secret dependencies.'
    );
  }

  return {
    requiredSecrets: names,
    requiredServices: [...requiredServices.keys()].sort(),
    serviceReferences: [...requiredServices.values()].sort((left, right) =>
      left.name.localeCompare(right.name)
    ),
    usesServices,
    usesNativeFetch,
    ...(firstIssue === undefined ? {} : { issue: firstIssue }),
  };
}

export function analyzeWomlScript(
  source: string,
  options: { readonly shadowedServices?: readonly string[] } = {}
): ScriptAnalysis {
  return analyzeScript(source, 'step', options);
}

export function analyzeWomlLifecycleScript(
  source: string,
  options: { readonly shadowedServices?: readonly string[] } = {}
): ScriptAnalysis {
  return analyzeScript(source, 'lifecycle', options);
}

export function analyzeWomlReusableScript(source: string): ScriptAnalysis {
  return analyzeScript(source, 'reusable-step');
}

export function analyzeWomlNotificationProviderScript(
  source: string
): ScriptAnalysis {
  return analyzeScript(source, 'notification-provider');
}
