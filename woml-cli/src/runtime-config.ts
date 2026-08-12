import { constants } from 'node:fs';
import { access, stat, statfs } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';

export const RUNTIME_CONFIG_PROFILE = 'woml.runtime-configuration/v1';
export const RUNTIME_CONFIG_MAX_BYTES = 64 * 1024;
export const RUNTIME_MIN_FREE_BYTES = 64 * 1024 * 1024;

const hostPattern = /^(?:localhost|(?:[a-zA-Z0-9](?:[a-zA-Z0-9.-]{0,251}[a-zA-Z0-9])?)|(?:\d{1,3}\.){3}\d{1,3}|[0-9a-fA-F:]+)$/;
const deploymentPattern = /^[a-z][a-z0-9]*(?:[._-][a-z0-9]+)*$/;

export interface RuntimeConfigurationV1 {
  readonly schemaVersion: 1;
  readonly deploymentName?: string;
  readonly statePath?: string;
  readonly public?: {
    readonly host?: string;
    readonly port?: number;
  };
  readonly admin?: {
    readonly host?: string;
    readonly port?: number;
  };
  readonly logging?: {
    readonly format?: 'text' | 'json';
    readonly level?: 'error' | 'warn' | 'info' | 'debug';
    readonly directory?: string;
  };
  readonly workers?: number;
  readonly shutdownTimeoutMs?: number;
  readonly observability?: {
    readonly health?: boolean;
    readonly metrics?: boolean;
  };
  readonly retention?: {
    readonly enabled: boolean;
    readonly succeededAfterDays?: number;
    readonly failedAfterDays?: number;
    readonly cancelledAfterDays?: number;
    readonly maintenanceHourUtc?: number;
  };
  readonly backup?: {
    readonly directory?: string;
  };
}

export interface RuntimeConfigurationOverrides {
  readonly deploymentName?: string;
  readonly statePath?: string;
  readonly publicHost?: string;
  readonly publicPort?: number;
  readonly adminHost?: string;
  readonly adminPort?: number;
  readonly logFormat?: 'text' | 'json';
  readonly logLevel?: 'error' | 'warn' | 'info' | 'debug';
  readonly logDirectory?: string;
  readonly workers?: number;
  readonly shutdownTimeoutMs?: number;
}

export interface ResolvedRuntimeConfigurationV1 {
  readonly profile: typeof RUNTIME_CONFIG_PROFILE;
  readonly schemaVersion: 1;
  readonly configPath?: string;
  readonly deploymentName: string;
  readonly statePath: string;
  readonly public: { readonly host: string; readonly port: number };
  readonly admin: { readonly host: string; readonly port: number };
  readonly logging: {
    readonly format: 'text' | 'json';
    readonly level: 'error' | 'warn' | 'info' | 'debug';
    readonly directory: string;
  };
  readonly workers: number;
  readonly shutdownTimeoutMs: number;
  readonly observability: {
    readonly health: boolean;
    readonly metrics: boolean;
  };
  readonly retention?: NonNullable<RuntimeConfigurationV1['retention']>;
  readonly backup?: { readonly directory: string };
  readonly sources: Readonly<
    Record<
      | 'deploymentName'
      | 'statePath'
      | 'public.host'
      | 'public.port'
      | 'admin.host'
      | 'admin.port'
      | 'logging.format'
      | 'logging.level'
      | 'logging.directory'
      | 'workers'
      | 'shutdownTimeoutMs',
      'cli' | 'environment' | 'config' | 'default'
    >
  >;
}

export interface RuntimePreflightV1 {
  readonly profile: 'woml.runtime-preflight/v1';
  readonly state: {
    readonly path: string;
    readonly existing: boolean;
    readonly writableAncestor: string;
    readonly availableBytes: number;
  };
  readonly logging: {
    readonly directory: string;
    readonly writableAncestor: string;
  };
  readonly backup?: {
    readonly directory: string;
    readonly writableAncestor: string;
  };
  readonly ports: {
    readonly public: string;
    readonly admin: string;
    readonly distinct: true;
  };
}

export class RuntimeConfigurationError extends Error {
  readonly code: string;

  constructor(code: string, message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = 'RuntimeConfigurationError';
    this.code = code;
  }
}

function object(value: unknown, subject: string): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw invalid(`${subject} must be a JSON object.`);
  }
  return value as Record<string, unknown>;
}

function only(value: Record<string, unknown>, allowed: readonly string[], subject: string): void {
  const unknown = Object.keys(value).filter(key => !allowed.includes(key));
  if (unknown.length > 0) {
    throw invalid(`${subject} contains unknown field${unknown.length === 1 ? '' : 's'}: ${unknown.join(', ')}.`);
  }
}

function invalid(message: string): RuntimeConfigurationError {
  return new RuntimeConfigurationError('WOML_RUNTIME_CONFIG_INVALID', message);
}

function optionalString(value: unknown, subject: string, max = 4096): string | undefined {
  if (value === undefined) return undefined;
  if (typeof value !== 'string' || value.length === 0 || value.length > max || value.includes('\0')) {
    throw invalid(`${subject} must be a non-empty string no longer than ${max} characters.`);
  }
  return value;
}

function optionalInteger(
  value: unknown,
  subject: string,
  minimum: number,
  maximum: number
): number | undefined {
  if (value === undefined) return undefined;
  if (!Number.isSafeInteger(value) || (value as number) < minimum || (value as number) > maximum) {
    throw invalid(`${subject} must be an integer from ${minimum} through ${maximum}.`);
  }
  return value as number;
}

function optionalBoolean(value: unknown, subject: string): boolean | undefined {
  if (value === undefined) return undefined;
  if (typeof value !== 'boolean') throw invalid(`${subject} must be true or false.`);
  return value;
}

function enumValue<T extends string>(
  value: unknown,
  subject: string,
  values: readonly T[]
): T | undefined {
  if (value === undefined) return undefined;
  if (typeof value !== 'string' || !values.includes(value as T)) {
    throw invalid(`${subject} must be one of: ${values.join(', ')}.`);
  }
  return value as T;
}

function host(value: unknown, subject: string): string | undefined {
  const parsed = optionalString(value, subject, 253);
  if (parsed !== undefined && (!hostPattern.test(parsed) || /[\s/]/.test(parsed))) {
    throw invalid(`${subject} must be an IP address or hostname without a port.`);
  }
  return parsed;
}

function parseConfiguration(value: unknown): RuntimeConfigurationV1 {
  const root = object(value, 'runtime configuration');
  only(root, [
    'schemaVersion', 'deploymentName', 'statePath', 'public', 'admin', 'logging',
    'workers', 'shutdownTimeoutMs', 'observability', 'retention', 'backup',
  ], 'runtime configuration');
  if (root.schemaVersion !== 1) {
    throw new RuntimeConfigurationError(
      'WOML_RUNTIME_CONFIG_UNSUPPORTED',
      'runtime configuration schemaVersion must be 1.'
    );
  }
  const deploymentName = optionalString(root.deploymentName, 'deploymentName', 128);
  if (deploymentName !== undefined && !deploymentPattern.test(deploymentName)) {
    throw invalid('deploymentName must use lowercase letters, digits, dots, underscores, or hyphens and start with a letter.');
  }

  const publicConfig = root.public === undefined ? undefined : object(root.public, 'public');
  if (publicConfig !== undefined) only(publicConfig, ['host', 'port'], 'public');
  const adminConfig = root.admin === undefined ? undefined : object(root.admin, 'admin');
  if (adminConfig !== undefined) only(adminConfig, ['host', 'port'], 'admin');
  const logging = root.logging === undefined ? undefined : object(root.logging, 'logging');
  if (logging !== undefined) only(logging, ['format', 'level', 'directory'], 'logging');
  const observability = root.observability === undefined ? undefined : object(root.observability, 'observability');
  if (observability !== undefined) only(observability, ['health', 'metrics'], 'observability');
  const retention = root.retention === undefined ? undefined : object(root.retention, 'retention');
  if (retention !== undefined) {
    only(retention, ['enabled', 'succeededAfterDays', 'failedAfterDays', 'cancelledAfterDays', 'maintenanceHourUtc'], 'retention');
    if (typeof retention.enabled !== 'boolean') throw invalid('retention.enabled is required and must be true or false.');
  }
  const backup = root.backup === undefined ? undefined : object(root.backup, 'backup');
  if (backup !== undefined) {
    only(backup, ['directory'], 'backup');
    if (backup.directory === undefined) throw invalid('backup.directory is required when backup is configured.');
  }

  return {
    schemaVersion: 1,
    ...(deploymentName === undefined ? {} : { deploymentName }),
    ...(root.statePath === undefined ? {} : { statePath: optionalString(root.statePath, 'statePath')! }),
    ...(publicConfig === undefined ? {} : { public: {
      ...(publicConfig.host === undefined ? {} : { host: host(publicConfig.host, 'public.host')! }),
      ...(publicConfig.port === undefined ? {} : { port: optionalInteger(publicConfig.port, 'public.port', 1, 65_535)! }),
    } }),
    ...(adminConfig === undefined ? {} : { admin: {
      ...(adminConfig.host === undefined ? {} : { host: host(adminConfig.host, 'admin.host')! }),
      ...(adminConfig.port === undefined ? {} : { port: optionalInteger(adminConfig.port, 'admin.port', 1, 65_535)! }),
    } }),
    ...(logging === undefined ? {} : { logging: {
      ...(logging.format === undefined ? {} : { format: enumValue(logging.format, 'logging.format', ['text', 'json'] as const)! }),
      ...(logging.level === undefined ? {} : { level: enumValue(logging.level, 'logging.level', ['error', 'warn', 'info', 'debug'] as const)! }),
      ...(logging.directory === undefined ? {} : { directory: optionalString(logging.directory, 'logging.directory')! }),
    } }),
    ...(root.workers === undefined ? {} : { workers: optionalInteger(root.workers, 'workers', 1, 256)! }),
    ...(root.shutdownTimeoutMs === undefined ? {} : { shutdownTimeoutMs: optionalInteger(root.shutdownTimeoutMs, 'shutdownTimeoutMs', 1_000, 300_000)! }),
    ...(observability === undefined ? {} : { observability: {
      ...(observability.health === undefined ? {} : { health: optionalBoolean(observability.health, 'observability.health')! }),
      ...(observability.metrics === undefined ? {} : { metrics: optionalBoolean(observability.metrics, 'observability.metrics')! }),
    } }),
    ...(retention === undefined ? {} : { retention: {
      enabled: retention.enabled as boolean,
      ...(retention.succeededAfterDays === undefined ? {} : { succeededAfterDays: optionalInteger(retention.succeededAfterDays, 'retention.succeededAfterDays', 1, 3_650)! }),
      ...(retention.failedAfterDays === undefined ? {} : { failedAfterDays: optionalInteger(retention.failedAfterDays, 'retention.failedAfterDays', 1, 3_650)! }),
      ...(retention.cancelledAfterDays === undefined ? {} : { cancelledAfterDays: optionalInteger(retention.cancelledAfterDays, 'retention.cancelledAfterDays', 1, 3_650)! }),
      ...(retention.maintenanceHourUtc === undefined ? {} : { maintenanceHourUtc: optionalInteger(retention.maintenanceHourUtc, 'retention.maintenanceHourUtc', 0, 23)! }),
    } }),
    ...(backup === undefined ? {} : { backup: {
      ...(backup.directory === undefined ? {} : { directory: optionalString(backup.directory, 'backup.directory')! }),
    } }),
  };
}

function envInteger(environment: Readonly<Record<string, string | undefined>>, name: string, minimum: number, maximum: number): number | undefined {
  const raw = environment[name];
  if (raw === undefined) return undefined;
  if (!/^\d+$/.test(raw)) throw invalid(`${name} must be an integer from ${minimum} through ${maximum}.`);
  return optionalInteger(Number(raw), name, minimum, maximum);
}

function value<T>(cli: T | undefined, environment: T | undefined, config: T | undefined, fallback: T): [T, 'cli' | 'environment' | 'config' | 'default'] {
  if (cli !== undefined) return [cli, 'cli'];
  if (environment !== undefined) return [environment, 'environment'];
  if (config !== undefined) return [config, 'config'];
  return [fallback, 'default'];
}

export async function resolveRuntimeConfiguration(
  rawConfigPath?: string,
  overrides: RuntimeConfigurationOverrides = {},
  environment: Readonly<Record<string, string | undefined>> = process.env,
  cwd = process.cwd()
): Promise<ResolvedRuntimeConfigurationV1> {
  let config: RuntimeConfigurationV1 | undefined;
  let configPath: string | undefined;
  let base = cwd;
  if (rawConfigPath !== undefined) {
    configPath = resolve(cwd, rawConfigPath);
    let file;
    try {
      file = await stat(configPath);
    } catch (error) {
      throw new RuntimeConfigurationError('WOML_RUNTIME_CONFIG_INVALID', `runtime configuration file does not exist: ${configPath}`, { cause: error });
    }
    if (!file.isFile() || file.size > RUNTIME_CONFIG_MAX_BYTES) {
      throw invalid(`runtime configuration must be a file no larger than ${RUNTIME_CONFIG_MAX_BYTES} bytes.`);
    }
    let decoded: unknown;
    try {
      decoded = JSON.parse(await Bun.file(configPath).text());
    } catch (error) {
      throw new RuntimeConfigurationError('WOML_RUNTIME_CONFIG_INVALID', 'runtime configuration must contain valid JSON.', { cause: error });
    }
    config = parseConfiguration(decoded);
    base = dirname(configPath);
  }

  const envDeployment = optionalString(environment.WOML_RUNTIME_DEPLOYMENT, 'WOML_RUNTIME_DEPLOYMENT', 128);
  if (envDeployment !== undefined && !deploymentPattern.test(envDeployment)) throw invalid('WOML_RUNTIME_DEPLOYMENT has an invalid deployment name.');
  const cliDeployment = optionalString(overrides.deploymentName, 'deploymentName', 128);
  if (cliDeployment !== undefined && !deploymentPattern.test(cliDeployment)) throw invalid('deploymentName has an invalid deployment name.');
  const cliStatePath = optionalString(overrides.statePath, 'statePath');
  const envStatePath = optionalString(environment.WOML_RUNTIME_STATE, 'WOML_RUNTIME_STATE');
  const cliPublicHost = host(overrides.publicHost, 'public.host');
  const envPublicHost = host(environment.WOML_RUNTIME_HOST, 'WOML_RUNTIME_HOST');
  const cliAdminHost = host(overrides.adminHost, 'admin.host');
  const envAdminHost = host(environment.WOML_RUNTIME_ADMIN_HOST, 'WOML_RUNTIME_ADMIN_HOST');
  const cliLogFormat = enumValue(overrides.logFormat, 'logging.format', ['text', 'json'] as const);
  const envLogFormat = enumValue(environment.WOML_RUNTIME_LOG_FORMAT, 'WOML_RUNTIME_LOG_FORMAT', ['text', 'json'] as const);
  const cliLogLevel = enumValue(overrides.logLevel, 'logging.level', ['error', 'warn', 'info', 'debug'] as const);
  const envLogLevel = enumValue(environment.WOML_RUNTIME_LOG_LEVEL, 'WOML_RUNTIME_LOG_LEVEL', ['error', 'warn', 'info', 'debug'] as const);
  const cliLogDirectory = optionalString(overrides.logDirectory, 'logging.directory');
  const envLogDirectory = optionalString(environment.WOML_RUNTIME_LOG_DIRECTORY, 'WOML_RUNTIME_LOG_DIRECTORY');
  const [deploymentName, deploymentSource] = value(cliDeployment, envDeployment, config?.deploymentName, 'default');
  const [rawStatePath, stateSource] = value(cliStatePath, envStatePath, config?.statePath, '.woml/state.sqlite');
  const [publicHost, publicHostSource] = value(cliPublicHost, envPublicHost, config?.public?.host, '127.0.0.1');
  const [publicPort, publicPortSource] = value(optionalInteger(overrides.publicPort, 'public.port', 1, 65_535), envInteger(environment, 'WOML_RUNTIME_PORT', 1, 65_535), config?.public?.port, 3_000);
  const [adminHost, adminHostSource] = value(cliAdminHost, envAdminHost, config?.admin?.host, '127.0.0.1');
  const [adminPort, adminPortSource] = value(optionalInteger(overrides.adminPort, 'admin.port', 1, 65_535), envInteger(environment, 'WOML_RUNTIME_ADMIN_PORT', 1, 65_535), config?.admin?.port, 3_001);
  const [logFormat, logFormatSource] = value(cliLogFormat, envLogFormat, config?.logging?.format, 'text' as const);
  const [logLevel, logLevelSource] = value(cliLogLevel, envLogLevel, config?.logging?.level, 'info' as const);
  const [rawLogDirectory, logDirectorySource] = value(cliLogDirectory, envLogDirectory, config?.logging?.directory, '.woml/logs');
  const [workers, workersSource] = value(optionalInteger(overrides.workers, 'workers', 1, 256), envInteger(environment, 'WOML_RUNTIME_WORKERS', 1, 256), config?.workers, 4);
  const [shutdownTimeoutMs, shutdownSource] = value(optionalInteger(overrides.shutdownTimeoutMs, 'shutdownTimeoutMs', 1_000, 300_000), envInteger(environment, 'WOML_RUNTIME_SHUTDOWN_TIMEOUT_MS', 1_000, 300_000), config?.shutdownTimeoutMs, 30_000);
  if (!['127.0.0.1', 'localhost', '::1'].includes(adminHost)) {
    throw invalid('admin.host must be a loopback address in Runtime Admin v1.');
  }
  if (publicHost === adminHost && publicPort === adminPort) {
    throw invalid('public and admin listeners cannot use the same host and port.');
  }
  const stateBase = stateSource === 'config' ? base : cwd;
  const logBase = logDirectorySource === 'config' ? base : cwd;
  const backupDirectory = config?.backup?.directory;
  return {
    profile: RUNTIME_CONFIG_PROFILE,
    schemaVersion: 1,
    ...(configPath === undefined ? {} : { configPath }),
    deploymentName,
    statePath: resolve(stateBase, rawStatePath),
    public: { host: publicHost, port: publicPort },
    admin: { host: adminHost, port: adminPort },
    logging: { format: logFormat, level: logLevel, directory: resolve(logBase, rawLogDirectory) },
    workers,
    shutdownTimeoutMs,
    observability: { health: config?.observability?.health ?? true, metrics: config?.observability?.metrics ?? true },
    ...(config?.retention === undefined ? {} : { retention: config.retention }),
    ...(backupDirectory === undefined ? {} : { backup: { directory: resolve(base, backupDirectory) } }),
    sources: {
      deploymentName: deploymentSource,
      statePath: stateSource,
      'public.host': publicHostSource,
      'public.port': publicPortSource,
      'admin.host': adminHostSource,
      'admin.port': adminPortSource,
      'logging.format': logFormatSource,
      'logging.level': logLevelSource,
      'logging.directory': logDirectorySource,
      workers: workersSource,
      shutdownTimeoutMs: shutdownSource,
    },
  };
}

async function writableAncestor(
  target: string,
  accessPath: (path: string, mode?: number) => Promise<void> = access
): Promise<{ path: string; existing: boolean }> {
  let candidate = target;
  let existing = true;
  while (true) {
    try {
      const entry = await stat(candidate);
      const directory = entry.isDirectory() ? candidate : dirname(candidate);
      await accessPath(directory, constants.R_OK | constants.W_OK);
      return { path: directory, existing };
    } catch (error) {
      if (error instanceof Error && 'code' in error && error.code === 'ENOENT') {
        existing = false;
        const parent = dirname(candidate);
        if (parent === candidate) throw error;
        candidate = parent;
        continue;
      }
      throw new RuntimeConfigurationError('WOML_DEPLOYMENT_PREFLIGHT_FAILED', `path is not readable and writable: ${target}`, { cause: error });
    }
  }
}

export async function preflightRuntimeConfiguration(
  configuration: ResolvedRuntimeConfigurationV1,
  dependencies: {
    readonly accessPath?: (path: string, mode?: number) => Promise<void>;
    readonly statFilesystem?: (
      path: string
    ) => Promise<{ readonly bavail: number | bigint; readonly bsize: number | bigint }>;
  } = {}
): Promise<RuntimePreflightV1> {
  const state = await writableAncestor(configuration.statePath, dependencies.accessPath);
  const logs = await writableAncestor(configuration.logging.directory, dependencies.accessPath);
  const backup = configuration.backup === undefined ? undefined : await writableAncestor(configuration.backup.directory, dependencies.accessPath);
  const filesystem = await (dependencies.statFilesystem ?? statfs)(state.path);
  const availableBytes = Number(filesystem.bavail) * Number(filesystem.bsize);
  if (!Number.isSafeInteger(availableBytes) || availableBytes < RUNTIME_MIN_FREE_BYTES) {
    throw new RuntimeConfigurationError(
      'WOML_DEPLOYMENT_PREFLIGHT_FAILED',
      `state filesystem must have at least ${RUNTIME_MIN_FREE_BYTES} available bytes.`
    );
  }
  return {
    profile: 'woml.runtime-preflight/v1',
    state: { path: configuration.statePath, existing: state.existing, writableAncestor: state.path, availableBytes },
    logging: { directory: configuration.logging.directory, writableAncestor: logs.path },
    ...(configuration.backup === undefined || backup === undefined ? {} : { backup: { directory: configuration.backup.directory, writableAncestor: backup.path } }),
    ports: {
      public: `${configuration.public.host}:${configuration.public.port}`,
      admin: `${configuration.admin.host}:${configuration.admin.port}`,
      distinct: true,
    },
  };
}
