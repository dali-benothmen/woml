# Installation

## Prerequisites

Before installing Cronflow, make sure you have:

- **Node.js** version 18 or higher
- **Bun** version 1.0.0 or higher (recommended for best performance)
- **npm**, **yarn**, or **pnpm** (optional, Bun is preferred)

## Quick Install

### Using Bun (Recommended)

```bash
bun add cronflow
```

### Using npm

```bash
npm install cronflow
```

### Using yarn

```bash
yarn add cronflow
```

### Using pnpm

```bash
pnpm add cronflow
```

## TypeScript Support

Cronflow is built with TypeScript and includes full type definitions. No additional `@types` package is needed.

```typescript
import { cronflow } from 'cronflow';
// Full TypeScript support out of the box
```

## System Requirements

### Supported Platforms

Cronflow supports the following platforms:

| Platform | Architecture | Status       |
| -------- | ------------ | ------------ |
| Linux    | x86_64       | ✅ Supported |
| Linux    | aarch64      | ✅ Supported |
| macOS    | x86_64       | ✅ Supported |
| macOS    | aarch64      | ✅ Supported |
| Windows  | x86_64       | ✅ Supported |
| Windows  | aarch64      | ✅ Supported |

### Runtime Requirements

- **Node.js**: 18.0.0 or higher
- **Bun**: 1.0.0 or higher (recommended)
- **Memory**: Minimum 512MB RAM
- **Disk**: 50MB free space
