#!/usr/bin/env node

/**
 * Build and install Cronflow core locally for testing
 *
 * This script:
 * 1. Detects the current OS and architecture
 * 2. Builds the Rust core for the current platform
 * 3. Creates the appropriate @cronflow package in node_modules
 * 4. Copies the .node file and creates package.json
 */

const { execSync } = require('child_process');
const fs = require('fs');
const path = require('path');
const os = require('os');

// Colors for console output
const colors = {
  reset: '\x1b[0m',
  bright: '\x1b[1m',
  green: '\x1b[32m',
  blue: '\x1b[34m',
  yellow: '\x1b[33m',
  red: '\x1b[31m',
};

function log(message, color = colors.reset) {
  console.log(`${color}${message}${colors.reset}`);
}

function getPlatformInfo() {
  const platform = os.platform();
  const arch = os.arch();

  // Map Node.js platform/arch to Rust target triple and NAPI platform name
  const platformMap = {
    'darwin-x64': {
      rustTarget: 'x86_64-apple-darwin',
      napiPlatform: 'darwin-x64',
      binaryName: 'core.darwin-x64.node',
      packageName: '@cronflow/darwin-x64',
    },
    'darwin-arm64': {
      rustTarget: 'aarch64-apple-darwin',
      napiPlatform: 'darwin-arm64',
      binaryName: 'core.darwin-arm64.node',
      packageName: '@cronflow/darwin-arm64',
    },
    'linux-x64': {
      rustTarget: 'x86_64-unknown-linux-gnu',
      napiPlatform: 'linux-x64-gnu',
      binaryName: 'core.linux-x64-gnu.node',
      packageName: '@cronflow/linux-x64-gnu',
    },
    'linux-arm64': {
      rustTarget: 'aarch64-unknown-linux-gnu',
      napiPlatform: 'linux-arm64-gnu',
      binaryName: 'core.linux-arm64-gnu.node',
      packageName: '@cronflow/linux-arm64-gnu',
    },
    'win32-x64': {
      rustTarget: 'x86_64-pc-windows-msvc',
      napiPlatform: 'win32-x64-msvc',
      binaryName: 'core.win32-x64-msvc.node',
      packageName: '@cronflow/win32-x64-msvc',
    },
    'win32-arm64': {
      rustTarget: 'aarch64-pc-windows-msvc',
      napiPlatform: 'win32-arm64-msvc',
      binaryName: 'core.win32-arm64-msvc.node',
      packageName: '@cronflow/win32-arm64-msvc',
    },
  };

  const key = `${platform}-${arch}`;
  const info = platformMap[key];

  if (!info) {
    throw new Error(`Unsupported platform: ${platform}-${arch}`);
  }

  return info;
}

function buildCore(debug = false) {
  const coreDir = path.join(process.cwd(), 'core');
  const buildMode = debug ? '' : '--release';

  log('\n📦 Building Rust core...', colors.blue);
  log(`   Mode: ${debug ? 'debug' : 'release'}`, colors.reset);

  try {
    execSync(`cd ${coreDir} && npm run build ${buildMode}`, {
      stdio: 'inherit',
    });
    log('✅ Core built successfully!', colors.green);
  } catch (error) {
    log('❌ Failed to build core', colors.red);
    throw error;
  }
}

function installCoreLocally(platformInfo, version = '0.11.3') {
  const projectRoot = process.cwd();
  const coreDir = path.join(projectRoot, 'core');
  const localBuildsDir = path.join(projectRoot, 'local-builds');
  const cronflowDir = path.join(localBuildsDir, '@cronflow');
  const platformDir = path.join(cronflowDir, platformInfo.napiPlatform);

  log('\n📂 Installing core locally...', colors.blue);

  // Create directories
  if (!fs.existsSync(cronflowDir)) {
    fs.mkdirSync(cronflowDir, { recursive: true });
    log(`   Created: ${cronflowDir}`, colors.reset);
  }

  if (fs.existsSync(platformDir)) {
    log(`   Removing old: ${platformDir}`, colors.yellow);
    fs.rmSync(platformDir, { recursive: true, force: true });
  }

  fs.mkdirSync(platformDir, { recursive: true });
  log(`   Created: ${platformDir}`, colors.reset);

  // Find the built .node file
  const possiblePaths = [
    path.join(coreDir, platformInfo.binaryName),
    path.join(coreDir, 'core.node'), // Fallback for local builds
  ];

  let sourceFile = null;
  for (const filePath of possiblePaths) {
    if (fs.existsSync(filePath)) {
      sourceFile = filePath;
      break;
    }
  }

  if (!sourceFile) {
    throw new Error(
      `Could not find built core binary. Tried:\n${possiblePaths.join('\n')}`
    );
  }

  // Copy the .node file
  const destFile = path.join(platformDir, platformInfo.binaryName);
  fs.copyFileSync(sourceFile, destFile);
  log(`   Copied: ${platformInfo.binaryName}`, colors.green);

  // Create package.json
  const packageJson = {
    name: platformInfo.packageName,
    version: version,
    description: `Cronflow native bindings for ${platformInfo.napiPlatform}`,
    main: platformInfo.binaryName,
    license: 'Apache-2.0',
    os: [platformInfo.napiPlatform.split('-')[0]],
    cpu: [platformInfo.napiPlatform.includes('arm64') ? 'arm64' : 'x64'],
    repository: {
      type: 'git',
      url: 'https://github.com/dali-benothmen/cronflow.git',
    },
    engines: {
      node: '>= 20',
    },
  };

  const packageJsonPath = path.join(platformDir, 'package.json');
  fs.writeFileSync(packageJsonPath, JSON.stringify(packageJson, null, 2));
  log(`   Created: package.json`, colors.green);

  log(`\n✅ Core installed locally at:`, colors.green);
  log(`   ${platformDir}`, colors.bright);
  log(`\n💡 You can now import and test with:`, colors.blue);
  log(
    `   const core = require('./local-builds/${platformInfo.packageName}');`,
    colors.reset
  );
  log(`\n📦 To use in node_modules (optional):`, colors.yellow);
  log(`   cp -r local-builds/@cronflow node_modules/`, colors.reset);
}

function main() {
  const args = process.argv.slice(2);
  const debug = args.includes('--debug') || args.includes('-d');
  const skipBuild = args.includes('--skip-build') || args.includes('-s');

  log('\n🚀 Cronflow Core Local Installer', colors.bright);
  log('='.repeat(50), colors.reset);

  try {
    // Detect platform
    const platformInfo = getPlatformInfo();
    log(`\n🔍 Detected platform:`, colors.blue);
    log(`   Platform: ${platformInfo.napiPlatform}`, colors.reset);
    log(`   Rust target: ${platformInfo.rustTarget}`, colors.reset);
    log(`   Binary: ${platformInfo.binaryName}`, colors.reset);

    // Build core (unless skipped)
    if (!skipBuild) {
      buildCore(debug);
    } else {
      log('\n⏭️  Skipping build (using existing binary)', colors.yellow);
    }

    // Install locally
    installCoreLocally(platformInfo);

    log('\n✨ Done! Core is ready for local testing.', colors.green);
    log('\n📝 Next steps:', colors.blue);
    log('   1. Import the core in your test file', colors.reset);
    log('   2. Test your async functions', colors.reset);
    log('   3. Run your tests: bun test or npm test', colors.reset);
    log('\n💡 Tips:', colors.yellow);
    log(
      '   • Use --debug flag for faster builds during development',
      colors.reset
    );
    log('   • Use --skip-build to only copy existing binary', colors.reset);
  } catch (error) {
    log(`\n❌ Error: ${error.message}`, colors.red);
    process.exit(1);
  }
}

main();
