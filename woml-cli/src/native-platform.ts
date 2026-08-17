export const womlNativeTargets = [
  'darwin-x64',
  'darwin-arm64',
  'win32-x64-msvc',
  'win32-arm64-msvc',
  'linux-x64-gnu',
  'linux-x64-musl',
  'linux-arm64-gnu',
  'linux-arm64-musl',
] as const;

export type WomlNativeTarget = (typeof womlNativeTargets)[number];

export interface WomlNativeTargetSpec {
  readonly target: WomlNativeTarget;
  readonly rustTarget: string;
  readonly os: 'darwin' | 'win32' | 'linux';
  readonly cpu: 'x64' | 'arm64';
  readonly libc?: 'glibc' | 'musl';
  readonly libraryName: string;
}

export const womlNativeTargetSpecs: Readonly<
  Record<WomlNativeTarget, WomlNativeTargetSpec>
> = {
  'darwin-x64': {
    target: 'darwin-x64',
    rustTarget: 'x86_64-apple-darwin',
    os: 'darwin',
    cpu: 'x64',
    libraryName: 'libwoml_core.dylib',
  },
  'darwin-arm64': {
    target: 'darwin-arm64',
    rustTarget: 'aarch64-apple-darwin',
    os: 'darwin',
    cpu: 'arm64',
    libraryName: 'libwoml_core.dylib',
  },
  'win32-x64-msvc': {
    target: 'win32-x64-msvc',
    rustTarget: 'x86_64-pc-windows-msvc',
    os: 'win32',
    cpu: 'x64',
    libraryName: 'woml_core.dll',
  },
  'win32-arm64-msvc': {
    target: 'win32-arm64-msvc',
    rustTarget: 'aarch64-pc-windows-msvc',
    os: 'win32',
    cpu: 'arm64',
    libraryName: 'woml_core.dll',
  },
  'linux-x64-gnu': {
    target: 'linux-x64-gnu',
    rustTarget: 'x86_64-unknown-linux-gnu',
    os: 'linux',
    cpu: 'x64',
    libc: 'glibc',
    libraryName: 'libwoml_core.so',
  },
  'linux-x64-musl': {
    target: 'linux-x64-musl',
    rustTarget: 'x86_64-unknown-linux-musl',
    os: 'linux',
    cpu: 'x64',
    libc: 'musl',
    libraryName: 'libwoml_core.so',
  },
  'linux-arm64-gnu': {
    target: 'linux-arm64-gnu',
    rustTarget: 'aarch64-unknown-linux-gnu',
    os: 'linux',
    cpu: 'arm64',
    libc: 'glibc',
    libraryName: 'libwoml_core.so',
  },
  'linux-arm64-musl': {
    target: 'linux-arm64-musl',
    rustTarget: 'aarch64-unknown-linux-musl',
    os: 'linux',
    cpu: 'arm64',
    libc: 'musl',
    libraryName: 'libwoml_core.so',
  },
};

export function nativePackageName(target: WomlNativeTarget): string {
  return `@woml-org/cli-${target}`;
}

export function nativePackageBinaryName(target: WomlNativeTarget): string {
  return `woml-core.${target}.node`;
}

export function localNativeBinaryName(
  platform: string,
  architecture: string,
): string {
  if (!['win32', 'darwin', 'linux'].includes(platform)) {
    throw new Error(`WOML does not support native builds for ${platform}.`);
  }
  if (!['x64', 'arm64'].includes(architecture)) {
    throw new Error(`WOML does not support native builds for ${architecture}.`);
  }
  return `woml-core.${platform}-${architecture}.node`;
}

export function nativeTargetForRuntime(
  platform: string,
  architecture: string,
  linuxLibc: 'glibc' | 'musl' = 'glibc',
): WomlNativeTarget {
  const cpu = architecture === 'x64' || architecture === 'arm64'
    ? architecture
    : undefined;
  if (cpu === undefined) {
    throw new Error(`WOML does not support native builds for ${architecture}.`);
  }
  if (platform === 'darwin') return `darwin-${cpu}`;
  if (platform === 'win32') return `win32-${cpu}-msvc`;
  if (platform === 'linux') {
    return `linux-${cpu}-${linuxLibc === 'musl' ? 'musl' : 'gnu'}`;
  }
  throw new Error(`WOML does not support native builds for ${platform}.`);
}

export function detectLinuxLibc(): 'glibc' | 'musl' {
  if (process.platform !== 'linux') return 'glibc';
  const report = process.report?.getReport?.() as
    | { readonly header?: { readonly glibcVersionRuntime?: unknown } }
    | undefined;
  return typeof report?.header?.glibcVersionRuntime === 'string'
    ? 'glibc'
    : 'musl';
}
