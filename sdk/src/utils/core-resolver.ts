import * as path from 'path';
import * as fs from 'fs';

export interface CoreResolution {
  path: string;
  found: boolean;
  error?: string;
  module?: any;
}

export function resolveCoreNode(): CoreResolution {
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

  return {
    path: possiblePaths[0],
    found: false,
    error: `Core.node not found in any of the expected locations. Tried: ${possiblePaths.join(', ')}`,
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
