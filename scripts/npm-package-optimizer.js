#!/usr/bin/env node

const fs = require('fs');
const path = require('path');
const { execSync } = require('child_process');

function optimizeForNpmPublishing() {
  console.log('📦 Optimizing package for npm publishing...\n');

  const distPath = path.join(process.cwd(), 'dist');
  const packagePath = path.join(process.cwd(), 'package-optimized');

  // Check if dist directory exists
  if (!fs.existsSync(distPath)) {
    console.error('❌ dist directory not found. Run build first.');
    process.exit(1);
  }

  try {
    // Remove previous optimized package
    if (fs.existsSync(packagePath)) {
      fs.rmSync(packagePath, { recursive: true, force: true });
    }

    // Create optimized package directory
    fs.mkdirSync(packagePath, { recursive: true });

    // Copy essential files for npm publishing
    copyEssentialFiles(packagePath);

    // Create npm package archive
    createNpmPackage(packagePath);

    // Generate final report
    generateFinalReport(distPath, packagePath);

    console.log('\n✅ NPM package optimization completed!');
    console.log('📦 Ready for publishing with minimal package size.');
  } catch (error) {
    console.error('❌ Optimization failed:', error.message);
    process.exit(1);
  }
}

function copyEssentialFiles(packagePath) {
  console.log('📁 Copying essential files for npm publishing...');

  // Copy dist directory (this will include dist/core/core.node)
  const distDest = path.join(packagePath, 'dist');
  fs.mkdirSync(distDest, { recursive: true });
  copyDirectory(path.join(process.cwd(), 'dist'), distDest);

  // Copy essential files from root
  const essentialFiles = [
    'package.json',
    'README.md',
    'LICENSE',
    'CHANGELOG.md',
  ];

  essentialFiles.forEach(file => {
    const srcPath = path.join(process.cwd(), file);
    const destPath = path.join(packagePath, file);

    if (fs.existsSync(srcPath)) {
      fs.copyFileSync(srcPath, destPath);
      console.log(`   ✅ Copied ${file}`);
    } else {
      console.log(`   ⚠️  ${file} not found, skipping`);
    }
  });

  // Note: core.node is now properly located in dist/core/core.node
  // No need to copy it to root as cronflow.ts expects it in core/ directory
  console.log('   ✅ Core.node structure maintained in dist/core/');
}

function copyDirectory(src, dest) {
  const items = fs.readdirSync(src);

  items.forEach(item => {
    const srcPath = path.join(src, item);
    const destPath = path.join(dest, item);
    const stats = fs.statSync(srcPath);

    if (stats.isDirectory()) {
      fs.mkdirSync(destPath, { recursive: true });
      copyDirectory(srcPath, destPath);
    } else {
      fs.copyFileSync(srcPath, destPath);
    }
  });
}

function createNpmPackage(packagePath) {
  console.log('🗜️  Creating npm package archive...');

  try {
    // Create tar.gz archive (npm's preferred format)
    const archivePath = path.join(
      process.cwd(),
      'node-cronflow-package.tar.gz'
    );
    const command = `tar -czf ${archivePath} -C ${process.cwd()} package-optimized/`;
    execSync(command, { stdio: 'pipe' });

    const size = fs.statSync(archivePath).size;
    console.log(`   ✅ npm package: ${formatBytes(size)}`);

    // Also create a zip for alternative distribution
    const zipPath = path.join(process.cwd(), 'node-cronflow-package.zip');
    try {
      const zipCommand = `cd ${packagePath} && zip -r ${zipPath} .`;
      execSync(zipCommand, { stdio: 'pipe' });

      const zipSize = fs.statSync(zipPath).size;
      console.log(`   ✅ zip package: ${formatBytes(zipSize)}`);
    } catch (zipError) {
      console.log(
        '   ⚠️  zip package creation failed (zip command not available)'
      );
    }
  } catch (error) {
    console.error('   ❌ Package creation failed:', error.message);
    throw error;
  }
}

function generateFinalReport(originalPath, packagePath) {
  console.log('\n📊 Final Package Analysis:');

  const originalSize = getDirectorySize(originalPath);
  const packageSize = getDirectorySize(packagePath);

  console.log(`   Original dist: ${formatBytes(originalSize)}`);
  console.log(`   Optimized package: ${formatBytes(packageSize)}`);

  // Check archive sizes
  const archives = [
    'node-cronflow-package.tar.gz',
    'node-cronflow-package.zip',
  ];
  archives.forEach(archive => {
    const archivePath = path.join(process.cwd(), archive);
    if (fs.existsSync(archivePath)) {
      const size = fs.statSync(archivePath).size;
      const compressionRatio = ((1 - size / packageSize) * 100).toFixed(1);
      console.log(
        `   ${archive}: ${formatBytes(size)} (${compressionRatio}% compression)`
      );
    }
  });

  // Show package structure
  console.log('\n📁 Package Structure:');
  showDirectoryStructure(packagePath, '   ');
}

function showDirectoryStructure(dirPath, prefix = '') {
  const items = fs.readdirSync(dirPath);

  items.forEach((item, index) => {
    const itemPath = path.join(dirPath, item);
    const stats = fs.statSync(itemPath);
    const isLast = index === items.length - 1;
    const connector = isLast ? '└── ' : '├── ';

    if (stats.isDirectory()) {
      console.log(`${prefix}${connector}${item}/`);
      showDirectoryStructure(itemPath, prefix + (isLast ? '    ' : '│   '));
    } else {
      const size = formatBytes(stats.size);
      console.log(`${prefix}${connector}${item} (${size})`);
    }
  });
}

function getDirectorySize(dirPath) {
  let totalSize = 0;

  function calculateSize(currentPath) {
    const stats = fs.statSync(currentPath);

    if (stats.isDirectory()) {
      const files = fs.readdirSync(currentPath);
      files.forEach(file => {
        calculateSize(path.join(currentPath, file));
      });
    } else {
      totalSize += stats.size;
    }
  }

  calculateSize(dirPath);
  return totalSize;
}

function formatBytes(bytes) {
  if (bytes === 0) return '0 Bytes';
  const k = 1024;
  const sizes = ['Bytes', 'KB', 'MB', 'GB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
}

// Run optimization
optimizeForNpmPublishing();
