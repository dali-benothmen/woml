export interface WomlModuleTestRuntime {
  readonly services: Readonly<Record<string, unknown>>;
}

let activeRuntime = false;

function deeplyReadonly(
  value: unknown,
  seen = new WeakMap<object, unknown>()
): unknown {
  if (
    (typeof value !== 'object' && typeof value !== 'function') ||
    value === null
  ) {
    return value;
  }
  const object = value as object;
  const existing = seen.get(object);
  if (existing !== undefined) return existing;
  const proxy = new Proxy(object, {
    set: () => false,
    defineProperty: () => false,
    deleteProperty: () => false,
    get(target, property, receiver) {
      return deeplyReadonly(Reflect.get(target, property, receiver), seen);
    },
  });
  seen.set(object, proxy);
  return proxy;
}

export async function withWomlModuleTestRuntime<T>(
  runtime: WomlModuleTestRuntime,
  test: () => T | Promise<T>
): Promise<T> {
  if (activeRuntime) {
    throw new Error('WOML module test runtimes cannot overlap in one process.');
  }
  activeRuntime = true;
  const previous = Object.getOwnPropertyDescriptor(globalThis, 'services');
  Object.defineProperty(globalThis, 'services', {
    configurable: true,
    enumerable: true,
    writable: false,
    value: deeplyReadonly(runtime.services),
  });
  try {
    return await test();
  } finally {
    if (previous === undefined)
      delete (globalThis as Record<string, unknown>).services;
    else Object.defineProperty(globalThis, 'services', previous);
    activeRuntime = false;
  }
}
