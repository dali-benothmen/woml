#!/usr/bin/env node

import { execSync } from 'child_process';
import { readdirSync, statSync } from 'fs';
import { join } from 'path';

console.log('📦 Bundle Analysis Report\n');

// Get total size
try {
  const totalSize = execSync('du -sh dist/', { encoding: 'utf8' }).trim();
  console.log(`📊 Total Bundle Size: ${totalSize}`);
} catch (error) {
  console.log('❌ Could not get total size');
}

// Analyze JavaScript files
console.log('\n📁 JavaScript Files:');
try {
  const jsFiles = execSync('find dist/ -name "*.js" -exec wc -c {} +', {
    encoding: 'utf8',
  });
  const lines = jsFiles.trim().split('\n');

  let totalJsSize = 0;
  const fileSizes = [];

  lines.forEach(line => {
    if (line.includes('total')) {
      totalJsSize = parseInt(line.split(' ')[0]);
    } else if (line.trim()) {
      const [size, path] = line.trim().split(' ');
      if (size && path) {
        fileSizes.push({ size: parseInt(size), path });
      }
    }
  });

  // Sort by size (largest first)
  fileSizes.sort((a, b) => b.size - a.size);

  fileSizes.forEach(({ size, path }) => {
    const sizeKB = (size / 1024).toFixed(2);
    console.log(`  ${path}: ${sizeKB} KB`);
  });

  console.log(
    `\n📈 Total JavaScript Size: ${(totalJsSize / 1024).toFixed(2)} KB`
  );
} catch (error) {
  console.log('❌ Could not analyze JavaScript files');
}

// Analyze TypeScript declaration files
console.log('\n📄 TypeScript Declaration Files:');
try {
  const dtsFiles = execSync('find dist/ -name "*.d.ts" -exec wc -l {} +', {
    encoding: 'utf8',
  });
  const lines = dtsFiles.trim().split('\n');

  let totalDtsLines = 0;
  const fileLines = [];

  lines.forEach(line => {
    if (line.includes('total')) {
      totalDtsLines = parseInt(line.split(' ')[0]);
    } else if (line.trim()) {
      const [lines, path] = line.trim().split(' ');
      if (lines && path) {
        fileLines.push({ lines: parseInt(lines), path });
      }
    }
  });

  // Sort by lines (largest first)
  fileLines.sort((a, b) => b.lines - a.lines);

  fileLines.forEach(({ lines, path }) => {
    console.log(`  ${path}: ${lines} lines`);
  });

  console.log(`\n📈 Total TypeScript Declaration Lines: ${totalDtsLines}`);
} catch (error) {
  console.log('❌ Could not analyze TypeScript declaration files');
}

// Check for source maps
console.log('\n🗺️ Source Maps:');
try {
  const sourceMaps = execSync('find dist/ -name "*.js.map" | wc -l', {
    encoding: 'utf8',
  }).trim();
  console.log(`  Generated ${sourceMaps} source map files`);
} catch (error) {
  console.log('❌ Could not check source maps');
}

// Directory structure
console.log('\n📂 Directory Structure:');
function printDirectoryStructure(
  dir,
  prefix = '',
  maxDepth = 3,
  currentDepth = 0
) {
  if (currentDepth >= maxDepth) return;

  try {
    const items = readdirSync(dir);
    items.forEach((item, index) => {
      const isLast = index === items.length - 1;
      const path = join(dir, item);
      const stat = statSync(path);

      if (stat.isDirectory()) {
        console.log(`${prefix}${isLast ? '└── ' : '├── '}${item}/`);
        printDirectoryStructure(
          path,
          prefix + (isLast ? '    ' : '│   '),
          maxDepth,
          currentDepth + 1
        );
      } else {
        const size = (stat.size / 1024).toFixed(2);
        console.log(`${prefix}${isLast ? '└── ' : '├── '}${item} (${size} KB)`);
      }
    });
  } catch (error) {
    console.log(`${prefix}❌ Could not read directory`);
  }
}

printDirectoryStructure('dist');

console.log('\n✅ Bundle analysis complete!');
console.log('\n💡 Tips:');
console.log('  - Use --minify flag for production builds');
console.log('  - Consider tree shaking for unused code elimination');
console.log('  - Monitor bundle size over time');
