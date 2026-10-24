export const publicJavaScriptFiles = [
  'dist/cli.js',
  'dist/script-host.js',
  'dist/script-host-worker.js',
  'dist/notification-provider-host.js',
  'dist/custom-notification-provider-host.js',
  'dist/custom-notification-provider-worker.js',
  'dist/opentelemetry-runtime.js',
  'dist/opentelemetry-sdk.js',
] as const;

export const publicSourceMapFiles = publicJavaScriptFiles.map(
  path => `${path}.map`,
);

export const publicSlackFiles = [
  'slack/README.md',
  'slack/manifest.json',
] as const;

export const publicProjectFiles = [
  'schemas/project-check.v1.schema.json',
  'schemas/woml-project.v1.schema.json',
  'templates/woml.project.json',
  'templates/workflows/hello.woml',
] as const;

export const publicPackageFiles = [
  ...publicJavaScriptFiles,
  ...publicSlackFiles,
  ...publicProjectFiles,
  'README.md',
  'LICENSE',
  'NOTICE.md',
] as const;

export const packedPackageFiles = [
  ...publicPackageFiles,
  'package.json',
] as const;
