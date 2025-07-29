import * as path from 'path';
import * as fs from 'fs';

export interface CoreResolution {
  path: string;
  found: boolean;
  error?: string;
  module?: any;
}

/**
 * Robust core.node resolution function that tries multiple possible locations
 * and provides detailed debugging information
 */
export function resolveCoreNode(): CoreResolution {
  const possiblePaths = [
    // Development: relative to current file
    path.join(__dirname, '..', '..', 'core', 'core.node'),
    // Production: relative to dist directory
    path.join(__dirname, '..', '..', '..', 'core', 'core.node'),
    // NPM package: in dist/core
    path.join(__dirname, '..', '..', 'core', 'core.node'),
    // Alternative npm package structure
    path.join(__dirname, '..', '..', '..', 'dist', 'core', 'core.node'),
    // Root level (for development)
    path.join(process.cwd(), 'core', 'core.node'),
    // Package root level
    path.join(process.cwd(), 'dist', 'core', 'core.node'),
    // Node modules path
    path.join(
      process.cwd(),
      'node_modules',
      'cronflow',
      'dist',
      'core',
      'core.node'
    ),
    // Global installation
    path.join(process.cwd(), 'node_modules', 'cronflow', 'core', 'core.node'),
  ];

  // Add platform-specific paths
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

  // Try each path
  for (const corePath of possiblePaths) {
    try {
      if (fs.existsSync(corePath)) {
        // Test if the file is actually loadable
        const testModule = require(corePath);
        if (testModule && typeof testModule === 'object') {
          return { path: corePath, found: true, module: testModule };
        }
      }
    } catch (error) {
      // Continue to next path
    }
  }

  // If no path works, return the first path for error reporting
  return {
    path: possiblePaths[0],
    found: false,
    error: `Core.node not found in any of the expected locations. Tried: ${possiblePaths.join(', ')}`,
  };
}

/**
 * Get detailed environment information for debugging
 */
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

/**
 * Load core module with comprehensive error handling and debugging
 */
export function loadCoreModule(): { core: any; resolution: CoreResolution } {
  const resolution = resolveCoreNode();

  if (resolution.found && resolution.module) {
    console.log('✅ Core loaded from:', resolution.path);
    console.log('📋 Environment info:', getEnvironmentInfo());
    return { core: resolution.module, resolution };
  } else {
    console.log('⚠️  Running in simulation mode (no Rust core)');
    console.log('📁 Environment details:');
    console.log('   Platform:', process.platform);
    console.log('   Architecture:', process.arch);
    console.log('   Node version:', process.version);
    console.log('   Current working directory:', process.cwd());
    console.log('   __dirname:', __dirname);
    console.log('   Error:', resolution.error);

    // Try to provide helpful debugging information
    try {
      const packageJsonPath = path.join(process.cwd(), 'package.json');
      if (fs.existsSync(packageJsonPath)) {
        const packageJson = JSON.parse(
          fs.readFileSync(packageJsonPath, 'utf8')
        );
        console.log('   Package name:', packageJson.name);
        console.log('   Package version:', packageJson.version);
      }
    } catch (e) {
      // Ignore package.json errors
    }

    return { core: null, resolution };
  }
}
