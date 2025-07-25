#!/usr/bin/env node

const fs = require('fs');
const path = require('path');
const { execSync } = require('child_process');

function compressDist() {
  console.log('🗜️  Compressing dist directory...\n');

  const distPath = path.join(process.cwd(), 'dist');
  const compressedPath = path.join(process.cwd(), 'dist.tar.gz');

  // Check if dist directory exists
  if (!fs.existsSync(distPath)) {
    console.error('❌ dist directory not found. Run build first.');
    process.exit(1);
  }

  try {
    // Create compressed archive
    const command = `tar -czf ${compressedPath} -C ${process.cwd()} dist/`;
    execSync(command, { stdio: 'inherit' });

    // Get file sizes
    const distSize = getDirectorySize(distPath);
    const compressedSize = fs.statSync(compressedPath).size;
    const compressionRatio = ((1 - compressedSize / distSize) * 100).toFixed(1);

    console.log('\n📊 Compression Results:');
    console.log(`   Original size: ${formatBytes(distSize)}`);
    console.log(`   Compressed size: ${formatBytes(compressedSize)}`);
    console.log(`   Compression ratio: ${compressionRatio}%`);
    console.log(`   Archive: ${compressedPath}`);

    console.log('\n✅ Dist compression completed successfully!');
  } catch (error) {
    console.error('❌ Compression failed:', error.message);
    process.exit(1);
  }
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

// Run compression
compressDist();
