#!/usr/bin/env node

const fs = require('fs');
const path = require('path');

// Bundle size limits (in bytes)
const SIZE_LIMITS = {
  'dist/dist/index.js': 1024 * 1024, // 1MB
  'dist/dist/sdk/index.js': 512 * 1024, // 512KB
  'dist/dist/services/index.js': 256 * 1024, // 256KB
  'dist/dist/core/core.node': 5 * 1024 * 1024, // 5MB
};

function getFileSize(filePath) {
  try {
    const stats = fs.statSync(filePath);
    return stats.size;
  } catch (error) {
    return 0;
  }
}

function formatBytes(bytes) {
  if (bytes === 0) return '0 Bytes';
  const k = 1024;
  const sizes = ['Bytes', 'KB', 'MB', 'GB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
}

function checkBundleSize() {
  console.log('🔍 Checking bundle sizes...\n');

  let totalSize = 0;
  let hasErrors = false;

  for (const [filePath, limit] of Object.entries(SIZE_LIMITS)) {
    const size = getFileSize(filePath);
    totalSize += size;

    const formattedSize = formatBytes(size);
    const formattedLimit = formatBytes(limit);

    if (size > limit) {
      console.log(
        `❌ ${filePath}: ${formattedSize} (exceeds limit: ${formattedLimit})`
      );
      hasErrors = true;
    } else {
      console.log(
        `✅ ${filePath}: ${formattedSize} (limit: ${formattedLimit})`
      );
    }
  }

  console.log(`\n📊 Total bundle size: ${formatBytes(totalSize)}`);

  if (hasErrors) {
    console.log(
      '\n❌ Bundle size check failed! Some files exceed their limits.'
    );
    process.exit(1);
  } else {
    console.log('\n✅ Bundle size check passed!');
  }
}

// Run the check
checkBundleSize();
