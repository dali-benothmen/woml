# State Management

Global and workflow-specific state management functions.

## `cronflow.getGlobalState(key, defaultValue?)`

Retrieves a value from global state.

```typescript
const value = await cronflow.getGlobalState('app.config', {});
```

## `cronflow.setGlobalState(key, value, options?)`

Sets a value in global state with optional TTL.

```typescript
await cronflow.setGlobalState(
  'temp.data',
  { value: 'temporary' },
  {
    ttl: '1h', // Expires in 1 hour
  }
);
```

## `cronflow.incrGlobalState(key, amount?)`

Increments a numeric value in global state.

```typescript
const newCount = await cronflow.incrGlobalState('user.count', 1);
```

## `cronflow.deleteGlobalState(key)`

Deletes a value from global state.

## `cronflow.getWorkflowState(workflowId, key, defaultValue?)`

Retrieves a value from workflow-specific state.

## `cronflow.setWorkflowState(workflowId, key, value, options?)`

Sets a value in workflow-specific state.

## `cronflow.incrWorkflowState(workflowId, key, amount?)`

Increments a numeric value in workflow-specific state.

## `cronflow.deleteWorkflowState(workflowId, key)`

Deletes a value from workflow-specific state.

## `cronflow.getStateStats()`

Returns statistics about state usage.

## `cronflow.cleanupExpiredState()`

Cleans up expired state entries.
