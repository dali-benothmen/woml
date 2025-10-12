# Cronflow Local Setup Guide

This guide will help you set up Cronflow locally for testing and development.

## 🆕 New in v0.9.0: Hidden Data Directory

**Important Update**: Starting with v0.9.0, Cronflow stores its database in a hidden `.cronflow/` directory instead of the project root. This keeps your project cleaner and follows standard conventions (like `.git/`, `.next/`, etc.).

**Before (< v0.9.0):**

```
my-project/
  ├── cronflow.db          ❌ Visible in root
  ├── package.json
  └── workflow.js
```

**After (>= v0.9.0):**

```
my-project/
  ├── .cronflow/           ✅ Hidden directory
  │   └── data.db
  ├── package.json
  └── workflow.js
```

**For new users**: Everything works automatically - the `.cronflow/` directory is created when you first run `cronflow.start()`.

**For existing users**: If you have an old `cronflow.db` file, see our [Migration Guide](./MIGRATION_V0.9.md) for options to:

- Start fresh (delete old database)
- Migrate existing data (move the file)
- Use a custom path (environment variable)

---

## 🎯 Quick Start (Testing Cronflow)

If you just want to try Cronflow in your own project:

Install from npm

```bash
# Create a new project
mkdir my-cronflow-project
cd my-cronflow-project
npm init -y

# Install cronflow
npm install cronflow express zod

# Create a simple workflow
cat > workflow.js << 'EOF'
const { cronflow } = require('cronflow');

// Define a simple workflow
const workflow = cronflow.define({
  id: 'hello-world',
  name: 'My First Workflow'
});

workflow
  .onWebhook('/webhooks/hello', { method: 'POST' })
  .step('greet', async (ctx) => {
    console.log('Received payload:', ctx.payload);
    return { message: `Hello, ${ctx.payload.name || 'World'}!` };
  })
  .action('log', (ctx) => {
    console.log('Result:', ctx.last);
  });

// Start the engine
cronflow.start({
  webhookServer: {
    host: '0.0.0.0',
    port: 3000
  }
}).then(() => {
  console.log('✅ Cronflow is running on http://localhost:3000');
  console.log('🧪 Test it: curl -X POST http://localhost:3000/webhooks/hello -H "Content-Type: application/json" -d \'{"name":"Developer"}\'');
});
EOF

# Run it
node workflow.js
```

**What happens**:

1. When you run `node workflow.js`, Cronflow automatically creates `.cronflow/data.db` in your current directory
2. The `.cronflow/` directory is hidden (starts with a dot) to keep your project clean
3. The database is initialized with the schema automatically
4. Your workflow is registered and ready to receive webhooks

---

## 🌍 Platform Support & Cross-Platform Installation

Cronflow uses native Rust binaries for maximum performance. We provide pre-built binaries for all major platforms.

### Supported Platforms

| Platform | Architecture          | Package Name                 |
| -------- | --------------------- | ---------------------------- |
| Windows  | x64                   | `@cronflow/win32-x64-msvc`   |
| Windows  | ARM64                 | `@cronflow/win32-arm64-msvc` |
| macOS    | Intel (x64)           | `@cronflow/darwin-x64`       |
| macOS    | Apple Silicon (ARM64) | `@cronflow/darwin-arm64`     |
| Linux    | x64 (GNU)             | `@cronflow/linux-x64-gnu`    |
| Linux    | x64 (musl)            | `@cronflow/linux-x64-musl`   |
| Linux    | ARM64 (GNU)           | `@cronflow/linux-arm64-gnu`  |
| Linux    | ARM64 (musl)          | `@cronflow/linux-arm64-musl` |

### How It Works

When you install `cronflow`, npm/bun automatically installs the correct platform-specific package as an optional dependency. For example:

- On **Windows x64**: `@cronflow/win32-x64-msvc` is installed
- On **macOS Apple Silicon**: `@cronflow/darwin-arm64` is installed
- On **Linux x64**: `@cronflow/linux-x64-gnu` is installed

**No compilation required!** The native binary is pre-built and ready to use.

### Troubleshooting Platform Issues

If Cronflow fails to load the native module:

1. **Check your platform and architecture:**

   ```bash
   node -e "console.log(process.platform, process.arch)"
   ```

2. **Verify the platform package is installed:**

   ```bash
   # On Windows x64
   npm list @cronflow/win32-x64-msvc

   # On macOS ARM64
   npm list @cronflow/darwin-arm64

   # On Linux x64
   npm list @cronflow/linux-x64-gnu
   ```

3. **Reinstall if the platform package is missing:**

   ```bash
   npm install --force cronflow
   ```

4. **Check for installation errors:**
   - Look for errors during `npm install` related to optional dependencies
   - Ensure you have a stable internet connection
   - Try clearing npm cache: `npm cache clean --force`

5. **Manual installation (last resort):**
   ```bash
   # Example for Windows x64
   npm install @cronflow/win32-x64-msvc
   ```

### Development Note

If you're developing Cronflow itself or need to build from source, see the [Development Setup](#-development-setup-contributing-to-cronflow) section below.

---

## 🛠️ Development Setup (Contributing to Cronflow)

If you want to contribute to Cronflow or modify its core:

### Prerequisites

- **Node.js** >= 18.0.0 OR **Bun** >= 1.0.0
- **Rust** >= 1.70.0 (for building the core)
- **Git**

### Step-by-Step Setup

```bash
# 1. Fork and clone the repository
git clone https://github.com/YOUR_USERNAME/cronflow.git
cd cronflow

# 2. Install dependencies
npm install  # or: bun install

# 3. Build the Rust core
cd core
cargo build --release
cd ..

# 4. Build the TypeScript SDK
npm run build  # or: bun run build

# 5. Run tests to verify everything works
npm test  # or: bun test
```

### Available Commands

```bash
# Development
npm run dev              # Watch mode for TypeScript
npm run dev:test         # Run tests in watch mode

# Building
npm run build            # Build TypeScript + Rust core
npm run build:core       # Build only Rust core
npm run build:prod       # Production build with optimizations

# Testing
npm test                 # Run all tests
npm run test:coverage    # Run tests with coverage

# Code Quality
npm run lint             # Check code style
npm run lint:fix         # Fix linting issues
npm run format           # Format code with Prettier

# Package Testing
npm run pack             # Create a local .tgz package
npm run test:package     # Test the packaged version
```

### Testing Your Changes Locally

After making changes, test them in a separate project:

```bash
# 1. Build and pack Cronflow
cd /path/to/cronflow
npm run build
npm pack  # Creates cronflow-X.X.X.tgz

# 2. Create a test project
mkdir ../test-cronflow
cd ../test-cronflow
npm init -y

# 3. Install your local build
npm install ../cronflow/cronflow-X.X.X.tgz

# 4. Create a test workflow (see Option 1 above)
# 5. Run and test your changes
```

## 🔍 Understanding the Database

**Important**: You don't need to manually create the database - it's automatic!

### How it works:

1. When you call `cronflow.start()`, the Rust core initializes
2. The core automatically creates `.cronflow/data.db` in a hidden directory
3. The `.cronflow/` directory keeps your project root clean
4. The database schema is applied automatically from `core/src/schema.sql`
5. Workflows are registered and stored in the database

### Database Location

By default, the database is created as `./.cronflow/data.db` in your current working directory.

```
my-project/
  ├── .cronflow/           ✅ Hidden directory
  │   └── data.db         Database file
  ├── .gitignore
  ├── node_modules/
  ├── package.json
  └── workflow.js
```

You can specify a custom location:

```javascript
// Default: uses ./.cronflow/data.db
cronflow.start();

// Custom location
cronflow.start({
  // dbPath is handled internally by the core
});

// The database includes:
// - workflows: Your workflow definitions
// - workflow_runs: Execution history
// - step_results: Individual step results
// - triggers: Configured triggers
```

### Troubleshooting Database Issues

If you don't see `.cronflow/` directory being created:

1. **Check if cronflow.start() is called**:

   ```javascript
   await cronflow.start(); // Make sure this is executed
   ```

2. **Check for errors**:

   ```javascript
   cronflow.start().catch(err => {
     console.error('Failed to start Cronflow:', err);
   });
   ```

3. **Verify Rust core is built**:

   ```bash
   ls core/core.node  # Should exist after building
   ```

4. **Check write permissions**:
   - Ensure your process has write permissions in the current directory

5. **View hidden files**:
   ```bash
   ls -la  # Shows hidden directories starting with .
   ```

## 🧪 Example Workflows

### Simple Webhook Workflow

```javascript
const { cronflow } = require('cronflow');

const workflow = cronflow.define({
  id: 'user-registration',
  name: 'User Registration Workflow',
});

workflow
  .onWebhook('/api/register', { method: 'POST' })
  .step('validate', async ctx => {
    // Validate user data
    const { email, name } = ctx.payload;
    if (!email || !name) {
      throw new Error('Missing required fields');
    }
    return { email, name, valid: true };
  })
  .step('save-user', async ctx => {
    // Save to database (pseudo-code)
    const userId = await db.users.create(ctx.last);
    return { userId, ...ctx.last };
  })
  .action('send-welcome-email', async ctx => {
    // Send welcome email (runs in background)
    await emailService.send({
      to: ctx.last.email,
      subject: 'Welcome!',
      body: `Hello ${ctx.last.name}!`,
    });
  });

cronflow.start({ webhookServer: { port: 3000 } });
```

### Manual Trigger Workflow

```javascript
const workflow = cronflow.define({
  id: 'data-processor',
  name: 'Data Processing Workflow',
});

workflow.step('process', async ctx => {
  const { data } = ctx.payload;
  const processed = data.map(item => ({
    ...item,
    processed: true,
    timestamp: new Date(),
  }));
  return { processed };
});

// Start the engine
await cronflow.start();

// Trigger manually
const runId = await cronflow.trigger('data-processor', {
  data: [{ id: 1 }, { id: 2 }],
});

console.log('Workflow started:', runId);
```

## 🆘 Common Issues

### Issue: "Cannot find module 'cronflow'"

**Solution**: Make sure Cronflow is installed:

```bash
npm install cronflow
# or for local development:
npm install ../path/to/cronflow-X.X.X.tgz
```

### Issue: ".cronflow directory is not created"

**Solution**: Ensure you're calling `cronflow.start()`:

```javascript
await cronflow.start(); // Database directory is created here
```

**Note**: The directory is hidden (starts with `.`). Use `ls -la` to see it.

### Issue: "No response from webhook"

**Solution**: Make sure the webhook server is started:

```javascript
cronflow.start({
  webhookServer: {
    port: 3000,
    host: '0.0.0.0', // Important for Docker/remote access
  },
});
```

### Issue: "Build fails with Rust errors"

**Solution**:

1. Ensure Rust is installed: `rustc --version`
2. Update Rust: `rustup update`
3. Clean and rebuild:
   ```bash
   cd core
   cargo clean
   cargo build --release
   ```

## 📚 Next Steps

1. **Read the Examples**: Check the [examples](./examples/examples.md) directory for real-world use cases
2. **API Reference**: See [docs/api-reference.md](./docs/api-reference.md) for detailed API documentation
3. **Contributing**: Read [CONTRIBUTING.md](./CONTRIBUTING.md) to contribute to the project
4. **Join the Community**: Participate in GitHub Discussions for questions and ideas

## 💡 Tips

- **Use TypeScript** for better type safety and autocomplete
- **Enable hot reload** with `nodemon` or `bun --watch` during development
- **Monitor performance** using the built-in monitoring features
- **Test workflows** using the testing utilities provided

---

**Still having issues?** Open a [GitHub Discussion](https://github.com/dali-benothmen/cronflow/discussions) and we'll help you out! 🚀
