# Core Status & Performance

Functions for monitoring core status and performance.

## `cronflow.isRustCoreAvailable()`

Checks if the Rust core is available.

## `cronflow.getCoreStatus()`

Returns the status of the Rust core.

## `cronflow.benchmark(options?)`

Runs performance benchmarks.

```typescript
const result = await cronflow.benchmark({
  iterations: 100,
  stepsPerWorkflow: 5,
  payloadSize: 1024,
  verbose: true,
});
```
