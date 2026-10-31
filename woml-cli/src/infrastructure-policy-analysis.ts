import type {
  InfrastructurePolicyBundleV1,
  InfrastructurePolicyRuleV1,
  PreparedInfrastructurePolicyV1,
} from './infrastructure-policy';

export interface InfrastructurePolicyDiagnosticV1 {
  readonly severity: 'warning';
  readonly code: 'duplicate_coverage' | 'unreachable_allow';
  readonly ruleIds: readonly string[];
  readonly message: string;
}

export interface InfrastructurePolicyStaticResultV1 {
  readonly workflowId: string;
  readonly stepId?: string;
  readonly capability: string;
  readonly operation: string;
  readonly status:
    'allowed' | 'denied' | 'requirements_missing' | 'unknown_until_runtime';
  readonly matchedRuleIds: readonly string[];
  readonly reason: string;
}

export function infrastructurePolicyDiagnostics(
  policy: PreparedInfrastructurePolicyV1
): readonly InfrastructurePolicyDiagnosticV1[] {
  const groups = new Map<string, InfrastructurePolicyRuleV1[]>();
  for (const rule of policy.bundle.rules) {
    const key = JSON.stringify(rule.match, Object.keys(rule.match).sort());
    groups.set(key, [...(groups.get(key) ?? []), rule]);
  }
  const diagnostics: InfrastructurePolicyDiagnosticV1[] = [];
  for (const rules of groups.values()) {
    if (rules.length < 2) continue;
    const ids = rules.map(rule => rule.id).sort();
    if (rules.some(rule => rule.decision === 'deny')) {
      const allows = rules.filter(rule => rule.decision === 'allow');
      if (allows.length > 0) {
        diagnostics.push({
          severity: 'warning',
          code: 'unreachable_allow',
          ruleIds: ids,
          message: `Allow rule coverage is shadowed by deny precedence for: ${ids.join(', ')}.`,
        });
        continue;
      }
    }
    diagnostics.push({
      severity: 'warning',
      code: 'duplicate_coverage',
      ruleIds: ids,
      message: `Multiple rules have identical match coverage: ${ids.join(', ')}.`,
    });
  }
  return diagnostics.sort((left, right) =>
    left.ruleIds[0]!.localeCompare(right.ruleIds[0]!)
  );
}

export function analyzeInfrastructurePolicySource(
  policy: InfrastructurePolicyBundleV1,
  workflowId: string,
  source: string,
  stepId?: string
): readonly InfrastructurePolicyStaticResultV1[] {
  const selected =
    stepId === undefined ? source : sourceForStep(source, stepId);
  const operations = new Map<
    string,
    { capability: string; operation: string }
  >();
  for (const match of selected.matchAll(
    /\bservices\.([A-Za-z][\w-]*)\.([A-Za-z][\w-]*)\s*\(/g
  )) {
    const capability = match[1]!;
    const operation = capability === 'state' && match[2] === 'setIfAbsent'
      ? 'set_if_absent'
      : match[2]!;
    operations.set(`${capability}\0${operation}`, { capability, operation });
  }
  if (/\bfetch\s*\(/.test(selected)) {
    operations.set('http\0request', {
      capability: 'http',
      operation: 'request',
    });
  }
  return [...operations.values()]
    .sort(
      (left, right) =>
        left.capability.localeCompare(right.capability) ||
        left.operation.localeCompare(right.operation)
    )
    .map(operation =>
      analyzeOperation(
        policy,
        workflowId,
        operation.capability,
        operation.operation,
        stepId
      )
    );
}

function analyzeOperation(
  policy: InfrastructurePolicyBundleV1,
  workflowId: string,
  capability: string,
  operation: string,
  stepId?: string
): InfrastructurePolicyStaticResultV1 {
  const candidates = policy.rules.filter(rule =>
    staticallyCouldMatch(rule, workflowId, capability, operation)
  );
  const certain = candidates.filter(rule => !hasRuntimeMatcher(rule));
  const certainDeny = certain.filter(rule => rule.decision === 'deny');
  if (certainDeny.length > 0) {
    return result(
      'denied',
      certainDeny,
      'A deny rule matches every value currently known.',
      workflowId,
      capability,
      operation,
      stepId
    );
  }
  const uncertainDeny = candidates.some(
    rule => rule.decision === 'deny' && hasRuntimeMatcher(rule)
  );
  const certainAllows = certain.filter(rule => rule.decision === 'allow');
  const requirements = certainAllows.filter(
    rule => rule.requires !== undefined
  );
  if (!uncertainDeny && requirements.length > 0) {
    return result(
      'requirements_missing',
      requirements,
      'Runtime approval, artifact, lock, or time evidence is required.',
      workflowId,
      capability,
      operation,
      stepId
    );
  }
  if (
    !uncertainDeny &&
    certainAllows.some(rule => rule.requires === undefined)
  ) {
    return result(
      'allowed',
      certainAllows,
      'The known operation fields match an unconditional allow rule.',
      workflowId,
      capability,
      operation,
      stepId
    );
  }
  return result(
    'unknown_until_runtime',
    candidates,
    'Target, arguments, evidence, effect class, or another runtime value is not statically known.',
    workflowId,
    capability,
    operation,
    stepId
  );
}

function staticallyCouldMatch(
  rule: InfrastructurePolicyRuleV1,
  workflowId: string,
  capability: string,
  operation: string
): boolean {
  const match = rule.match;
  return (
    (match.capabilities === undefined ||
      match.capabilities.includes(capability)) &&
    (match.operations === undefined || match.operations.includes(operation)) &&
    (match.workflows === undefined || match.workflows.includes(workflowId)) &&
    (match.executionModes === undefined ||
      match.executionModes.includes('normal'))
  );
}

function hasRuntimeMatcher(rule: InfrastructurePolicyRuleV1): boolean {
  const match = rule.match;
  return (
    match.targets !== undefined ||
    match.executables !== undefined ||
    match.argumentPrefix !== undefined ||
    match.effects !== undefined
  );
}

function result(
  status: InfrastructurePolicyStaticResultV1['status'],
  rules: readonly InfrastructurePolicyRuleV1[],
  reason: string,
  workflowId: string,
  capability: string,
  operation: string,
  stepId?: string
): InfrastructurePolicyStaticResultV1 {
  return {
    workflowId,
    ...(stepId === undefined ? {} : { stepId }),
    capability,
    operation,
    status,
    matchedRuleIds: rules.map(rule => rule.id).sort(),
    reason,
  };
}

function sourceForStep(source: string, stepId: string): string {
  const escaped = stepId.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const match = source.match(
    new RegExp(
      `<step\\b[^>]*\\bid=["']${escaped}["'][^>]*>([\\s\\S]*?)<\\/step>`,
      'i'
    )
  );
  return match?.[1] ?? '';
}
