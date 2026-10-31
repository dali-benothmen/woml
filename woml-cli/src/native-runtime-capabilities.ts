import type { RuntimeCapabilitiesV1 } from './rust-executor';

/** Keep the public JS options separate from the native decoder's frozen keys. */
export function encodeNativeRuntimeCapabilities(value: RuntimeCapabilitiesV1): string {
  const { infrastructurePolicy, emergencyDeny, ...capabilities } = value;
  return JSON.stringify({
    ...capabilities,
    ...(infrastructurePolicy === undefined ? {} : { infrastructure_policy: infrastructurePolicy }),
    ...(emergencyDeny === undefined ? {} : { emergency_deny: emergencyDeny }),
  });
}
