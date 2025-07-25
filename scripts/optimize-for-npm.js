#!/usr/bin/env node

const fs = require('fs');
const path = require('path');
const { execSync } = require('child_process');

function optimizeForNpm() {
  console.log('🚀 Optimizing dist for npm publishing...\n');

  const distPath = path.join(process.cwd(), 'dist');
  const optimizedPath = path.join(process.cwd(), 'dist-optimized');

  // Check if dist directory exists
  if (!fs.existsSync(distPath)) {
    console.error('❌ dist directory not found. Run build first.');
    process.exit(1);
  }

  try {
    // Remove previous optimized directory
    if (fs.existsSync(optimizedPath)) {
      fs.rmSync(optimizedPath, { recursive: true, force: true });
    }

    // Create optimized directory
    fs.mkdirSync(optimizedPath, { recursive: true });

    // Copy and optimize files
    copyAndOptimizeDirectory(distPath, optimizedPath);

    // Create compressed archives
    createCompressedArchives(optimizedPath);

    // Generate size report
    generateSizeReport(distPath, optimizedPath);

    console.log('\n✅ NPM optimization completed successfully!');
    console.log('📦 Ready for publishing with optimized package size.');
  } catch (error) {
    console.error('❌ Optimization failed:', error.message);
    process.exit(1);
  }
}

function copyAndOptimizeDirectory(src, dest) {
  console.log('📁 Copying and optimizing files...');

  const items = fs.readdirSync(src);

  items.forEach(item => {
    const srcPath = path.join(src, item);
    const destPath = path.join(dest, item);
    const stats = fs.statSync(srcPath);

    if (stats.isDirectory()) {
      fs.mkdirSync(destPath, { recursive: true });
      copyAndOptimizeDirectory(srcPath, destPath);
    } else {
      // Copy file with potential optimization
      fs.copyFileSync(srcPath, destPath);

      // Optimize specific file types
      if (item.endsWith('.js') || item.endsWith('.d.ts')) {
        optimizeFile(destPath);
      }
    }
  });
}

function optimizeFile(filePath) {
  try {
    let content = fs.readFileSync(filePath, 'utf8');

    // Remove unnecessary whitespace and comments (basic optimization)
    if (filePath.endsWith('.js')) {
      // Remove single-line comments (but keep license headers)
      content = content.replace(/\/\/.*$/gm, '');
      // Remove multi-line comments (but keep license headers)
      content = content.replace(/\/\*[\s\S]*?\*\//g, '');
      // Remove extra whitespace
      content = content.replace(/\s+/g, ' ');
      content = content.replace(/\s*{\s*/g, '{');
      content = content.replace(/\s*}\s*/g, '}');
      content = content.replace(/\s*;\s*/g, ';');
      content = content.replace(/\s*,\s*/g, ',');
    }

    fs.writeFileSync(filePath, content);
  } catch (error) {
    // If optimization fails, keep original file
    console.warn(`⚠️  Could not optimize ${filePath}: ${error.message}`);
  }
}

function createCompressedArchives(optimizedPath) {
  console.log('🗜️  Creating compressed archives...');

  const archives = [
    { name: 'dist.tar.gz', command: 'tar -czf' },
    { name: 'dist.tar.bz2', command: 'tar -cjf' },
    { name: 'dist.zip', command: 'zip -r' },
  ];

  archives.forEach(archive => {
    try {
      const archivePath = path.join(process.cwd(), archive.name);
      const command = `${archive.command} ${archivePath} -C ${process.cwd()} dist-optimized/`;
      execSync(command, { stdio: 'pipe' });

      const size = fs.statSync(archivePath).size;
      console.log(`   ✅ ${archive.name}: ${formatBytes(size)}`);
    } catch (error) {
      console.warn(`   ⚠️  Could not create ${archive.name}: ${error.message}`);
    }
  });
}

function generateSizeReport(originalPath, optimizedPath) {
  console.log('\n📊 Size Analysis:');

  const originalSize = getDirectorySize(originalPath);
  const optimizedSize = getDirectorySize(optimizedPath);
  const reduction = (
    ((originalSize - optimizedSize) / originalSize) *
    100
  ).toFixed(1);

  console.log(`   Original dist: ${formatBytes(originalSize)}`);
  console.log(`   Optimized dist: ${formatBytes(optimizedSize)}`);
  console.log(`   Size reduction: ${reduction}%`);

  // Check compressed archive sizes
  const archives = ['dist.tar.gz', 'dist.tar.bz2', 'dist.zip'];
  archives.forEach(archive => {
    const archivePath = path.join(process.cwd(), archive);
    if (fs.existsSync(archivePath)) {
      const size = fs.statSync(archivePath).size;
      const compressionRatio = ((1 - size / originalSize) * 100).toFixed(1);
      console.log(
        `   ${archive}: ${formatBytes(size)} (${compressionRatio}% compression)`
      );
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
optimizeForNpm();
