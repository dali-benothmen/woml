import * as path from 'path';
import * as fs from 'fs';

export interface CoreResolution {
  path: string;
  found: boolean;
  error?: string;
  module?: any;
}

/**
 * Maps process.platform and process.arch to NAPI-RS platform package names
 */
function getPlatformPackageName(): string | null {
  const platform = process.platform;
  const arch = process.arch;

  // Map platform + arch to NAPI-RS package naming convention
  const platformMap: Record<string, Record<string, string>> = {
    win32: {
      x64: '@cronflow/win32-x64-msvc',
      arm64: '@cronflow/win32-arm64-msvc',
    },
    darwin: {
      x64: '@cronflow/darwin-x64',
      arm64: '@cronflow/darwin-arm64',
    },
    linux: {
      x64: '@cronflow/linux-x64-gnu',
      arm64: '@cronflow/linux-arm64-gnu',
    },
  };

  return platformMap[platform]?.[arch] || null;
}

/**
 * Attempts to load the native module from platform-specific optional package
 */
function loadFromPlatformPackage(): CoreResolution | null {
  const packageName = getPlatformPackageName();
  
  if (!packageName) {
    return null;
  }

  try {
    // Try to require the platform-specific package
    const platformModule = require(packageName);
    
    if (platformModule && typeof platformModule === 'object') {
      return {
        path: packageName,
        found: true,
        module: platformModule,
      };
    }
  } catch (error) {
    // Platform package not installed or failed to load
    // This is expected during development or if installation failed
  }

  return null;
}

export function resolveCoreNode(): CoreResolution {
  // First, try to load from platform-specific optional package
  const platformResolution = loadFromPlatformPackage();
  if (platformResolution) {
    return platformResolution;
  }

  // Fall back to traditional path resolution for development/local builds
  const possiblePaths = [
    path.join(__dirname, '..', '..', 'core', 'core.node'),
    path.join(__dirname, '..', '..', '..', 'core', 'core.node'),
    path.join(__dirname, '..', '..', 'core', 'core.node'),
    path.join(__dirname, '..', '..', '..', 'dist', 'core', 'core.node'),
    path.join(process.cwd(), 'core', 'core.node'),
    path.join(process.cwd(), 'dist', 'core', 'core.node'),
    path.join(
      process.cwd(),
      'node_modules',
      'cronflow',
      'dist',
      'core',
      'core.node'
    ),
    path.join(process.cwd(), 'node_modules', 'cronflow', 'core', 'core.node'),
  ];

  const platform = process.platform;
  const arch = process.arch;

  if (platform === 'linux') {
    possiblePaths.push(
      path.join(__dirname, '..', '..', 'core', 'core.node'),
      path.join(__dirname, '..', '..', '..', 'core', 'core.node'),
      path.join(process.cwd(), 'node_modules', 'cronflow', 'core', 'core.node'),
      path.join(
        process.cwd(),
        'node_modules',
        'cronflow',
        'dist',
        'core',
        'core.node'
      )
    );
  } else if (platform === 'darwin') {
    possiblePaths.push(
      path.join(__dirname, '..', '..', 'core', 'core.node'),
      path.join(__dirname, '..', '..', '..', 'core', 'core.node'),
      path.join(process.cwd(), 'node_modules', 'cronflow', 'core', 'core.node'),
      path.join(
        process.cwd(),
        'node_modules',
        'cronflow',
        'dist',
        'core',
        'core.node'
      )
    );
  } else if (platform === 'win32') {
    possiblePaths.push(
      path.join(__dirname, '..', '..', 'core', 'core.node'),
      path.join(__dirname, '..', '..', '..', 'core', 'core.node'),
      path.join(process.cwd(), 'node_modules', 'cronflow', 'core', 'core.node'),
      path.join(
        process.cwd(),
        'node_modules',
        'cronflow',
        'dist',
        'core',
        'core.node'
      )
    );
  }

  for (const corePath of possiblePaths) {
    try {
      if (fs.existsSync(corePath)) {
        const testModule = require(corePath);
        if (testModule && typeof testModule === 'object') {
          return { path: corePath, found: true, module: testModule };
        }
      }
    } catch {}
  }

  // Provide helpful error message with platform package name
  const platformPackage = getPlatformPackageName();
  const errorMessage = platformPackage
    ? `Core.node not found. Expected platform package ${platformPackage} to be installed. Tried paths: ${possiblePaths.join(', ')}`
    : `Core.node not found for unsupported platform: ${platform}-${arch}. Tried: ${possiblePaths.join(', ')}`;

  return {
    path: possiblePaths[0],
    found: false,
    error: errorMessage,
  };
}

export function getEnvironmentInfo() {
  return {
    platform: process.platform,
    arch: process.arch,
    nodeVersion: process.version,
    cwd: process.cwd(),
    dirname: __dirname,
    env: {
      NODE_ENV: process.env.NODE_ENV,
      CRONFLOW_ENV: process.env.CRONFLOW_ENV,
    },
  };
}

export function loadCoreModule(): { core: any; resolution: CoreResolution } {
  const resolution = resolveCoreNode();

  if (resolution.found && resolution.module) {
    return { core: resolution.module, resolution };
  } else {
    try {
      const packageJsonPath = path.join(process.cwd(), 'package.json');
      if (fs.existsSync(packageJsonPath)) {
        const packageJson = JSON.parse(
          fs.readFileSync(packageJsonPath, 'utf8')
        );
      }
    } catch {}

    return { core: null, resolution };
  }
}
