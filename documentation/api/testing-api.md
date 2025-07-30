# Testing API

## `.test()`

Creates a test harness for the workflow.

```typescript
const testRun = await workflow
  .test()
  .trigger({ orderId: 'ord_123' })
  .expectStep('validate-order')
  .toSucceed()
  .run();
```

## `.advancedTest()`

Creates an advanced test harness with more features.
