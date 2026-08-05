import type { JsonObject, JsonValue } from './model';

export interface JsonViolation {
  readonly path: string;
  readonly reason: string;
}

function inspectJsonValue(
  value: unknown,
  path: string,
  ancestors: Set<object>,
): JsonViolation | undefined {
  if (value === null || typeof value === 'string' || typeof value === 'boolean') {
    return undefined;
  }
  if (typeof value === 'number') {
    return Number.isFinite(value)
      ? undefined
      : { path, reason: 'numbers must be finite' };
  }
  if (typeof value !== 'object') {
    return { path, reason: `${typeof value} is not a JSON value` };
  }
  if (ancestors.has(value)) {
    return { path, reason: 'circular references are not JSON values' };
  }

  ancestors.add(value);
  try {
    if (Array.isArray(value)) {
      const allowedKeys = new Set(['length']);
      for (let index = 0; index < value.length; index += 1) {
        const key = String(index);
        allowedKeys.add(key);
        if (!Object.prototype.hasOwnProperty.call(value, index)) {
          return { path: `${path}[${index}]`, reason: 'array holes are not JSON values' };
        }
        const issue = inspectJsonValue(value[index], `${path}[${index}]`, ancestors);
        if (issue !== undefined) return issue;
      }
      for (const key of Reflect.ownKeys(value)) {
        if (typeof key !== 'string' || !allowedKeys.has(key)) {
          return { path, reason: 'arrays must not contain custom properties' };
        }
      }
      return undefined;
    }

    const prototype = Object.getPrototypeOf(value);
    if (prototype !== Object.prototype && prototype !== null) {
      return { path, reason: 'objects must be plain JSON objects' };
    }

    for (const key of Reflect.ownKeys(value)) {
      if (typeof key !== 'string') {
        return { path, reason: 'symbol keys are not JSON object keys' };
      }
      const descriptor = Object.getOwnPropertyDescriptor(value, key);
      if (
        descriptor === undefined ||
        descriptor.enumerable !== true ||
        'get' in descriptor ||
        'set' in descriptor
      ) {
        return { path: `${path}.${key}`, reason: 'JSON fields must be enumerable data properties' };
      }
      const issue = inspectJsonValue(descriptor.value, `${path}.${key}`, ancestors);
      if (issue !== undefined) return issue;
    }
    return undefined;
  } finally {
    ancestors.delete(value);
  }
}

export function findJsonViolation(value: unknown): JsonViolation | undefined {
  return inspectJsonValue(value, '$', new Set());
}

export function isJsonObject(value: unknown): value is JsonObject {
  return (
    typeof value === 'object' &&
    value !== null &&
    !Array.isArray(value) &&
    findJsonViolation(value) === undefined
  );
}

export function cloneJson<T extends JsonValue>(value: T): T {
  return structuredClone(value) as T;
}

export function deepFreezeJson<T extends JsonValue>(value: T): T {
  if (typeof value !== 'object' || value === null) {
    return value;
  }
  if (Array.isArray(value)) {
    for (const item of value) deepFreezeJson(item);
  } else {
    for (const item of Object.values(value)) deepFreezeJson(item);
  }
  return Object.isFrozen(value) ? value : Object.freeze(value);
}
