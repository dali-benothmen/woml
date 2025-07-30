# Context Object

The context object (`ctx`) provides access to all workflow data and utilities.

## Properties

- `ctx.payload` - Data from the trigger that started the workflow
- `ctx.steps` - Outputs from all previously completed steps
- `ctx.run` - Metadata about the current run (`runId`, `workflowId`)
- `ctx.state` - Persistent state shared across workflow runs
- `ctx.last` - Output from the previous step (convenience property)
- `ctx.trigger` - Information about what triggered this workflow

## State Management

```typescript
// Set a value
await ctx.state.set('user-count', 42);

// Get a value
const count = await ctx.state.get('user-count', 0);

// Increment a value
const newCount = await ctx.state.incr('user-count', 1);

// Set with TTL
await ctx.state.set('temp-data', data, { ttl: '1h' });
```
