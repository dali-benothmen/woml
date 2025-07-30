# Configuration

## Workflow Options

| Option        | Type               | Default     | Description                                                  |
| ------------- | ------------------ | ----------- | ------------------------------------------------------------ |
| `timeout`     | `string \| number` | `'30m'`     | Maximum duration for the entire workflow run                 |
| `concurrency` | `number`           | `Infinity`  | Maximum number of concurrent runs allowed                    |
| `rateLimit`   | `object`           | -           | Limits execution frequency: `{ count: number, per: string }` |
| `queue`       | `string`           | `'default'` | Assigns workflow to a specific execution queue               |
| `version`     | `string`           | -           | Version string to manage multiple workflow versions          |
| `secrets`     | `object`           | -           | Configuration for fetching secrets from a vault              |

## Step Options

| Option    | Type               | Default                 | Description                                                    |
| --------- | ------------------ | ----------------------- | -------------------------------------------------------------- |
| `timeout` | `string \| number` | Inherited from workflow | Maximum duration for this step                                 |
| `retry`   | `object`           | Inherited from workflow | Retry configuration for this step                              |
| `cache`   | `object`           | -                       | Caching configuration: `{ key: (ctx) => string, ttl: string }` |
| `delay`   | `string \| number` | -                       | Delay before executing this step                               |
