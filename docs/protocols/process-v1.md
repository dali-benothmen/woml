# WOML Process v1

**Status:** Frozen by SP0  
**Capability:** `process`  
**Operation:** `run`  
**Transport:** Existing Capability Call v1  
**Authority:** Rust core

## Purpose

Process v1 is the versioned contract behind `services.process.run()`. It lets a
WOML script invoke one runtime-approved executable without turning the script
host into an unrestricted shell.

Process v1 adds no new script-host or generic capability envelope. A normalized
Process v1 request travels as `CapabilityCallRequest.input` with capability
`process`, operation `run`, input contract version `1`, and result contract
version `1`.

## Authoring API

```js
const result = await services.process.run({
  executable: "kubectl",
  args: ["get", "pods", "--namespace", "production"],
  cwd: "./infrastructure",
  env: { KUBECONFIG: secrets.KUBECONFIG },
  timeout: "30s",
  acceptExitCodes: [0],
  output: {
    stdout: "capture",
    stderr: "capture",
    maxBytes: 65536
  }
}, { name: "list-production-pods" });
```

The second argument uses the existing managed-operation naming contract and is
exactly `{ name: "stable-operation-name" }` when present.

The frozen public declarations are:

```ts
interface WomlProcessRunRequest {
  readonly executable: string;
  readonly args?: readonly string[];
  readonly cwd?: string;
  readonly env?: Readonly<Record<string, string>>;
  readonly timeout?: number | string;
  readonly acceptExitCodes?: readonly number[];
  readonly output?: Readonly<{
    stdout?: "capture" | "discard";
    stderr?: "capture" | "discard";
    maxBytes?: number;
  }>;
}

interface WomlProcessRunResult {
  readonly exitCode: number | null;
  readonly signal: string | null;
  readonly stdout: string;
  readonly stderr: string;
  readonly stdoutBytes: number;
  readonly stderrBytes: number;
  readonly stdoutTruncated: boolean;
  readonly stderrTruncated: boolean;
  readonly durationMs: number;
}
```

## Normalized request

The Bun facade applies defaults and emits:

```json
{
  "contract": "woml.process",
  "contractVersion": 1,
  "kind": "request",
  "executable": "kubectl",
  "args": ["get", "pods", "--namespace", "production"],
  "cwd": "./infrastructure",
  "env": {},
  "timeoutMs": 30000,
  "acceptExitCodes": [0],
  "output": {
    "stdout": "capture",
    "stderr": "capture",
    "maxBytes": 65536
  }
}
```

`executable` is a policy alias, not an author-controlled filesystem path. The
runtime maps the alias to one canonical approved executable.

`cwd` is relative to the policy's canonical project root. Absolute paths,
NUL bytes, and paths that resolve outside an allowed working-directory root are
rejected by the Rust authority.

## Defaults and limits

| Field | Default | Frozen Process v1 limit |
|---|---:|---:|
| `args` | `[]` | 256 arguments; 32,768 characters each |
| `cwd` | `.` | 4,096 characters before canonical validation |
| `env` | `{}` | 128 variables; 32,768 characters per value |
| `timeoutMs` | 30,000 | 86,400,000 |
| `acceptExitCodes` | `[0]` | 32 unique codes from 0 through 255 |
| stdout mode | `capture` | `capture` or `discard` |
| stderr mode | `capture` | `capture` or `discard` |
| output `maxBytes` | 65,536 | 262,144 per stream |

The capability envelope timeout is the lower of:

1. Process timeout plus a bounded supervision allowance;
2. remaining step deadline;
3. generic Capability Call v1 maximum.

Rust applies the lower of the request value and the active Process Policy v1
limit.

## Result

A successfully accepted exit returns:

```json
{
  "contract": "woml.process",
  "contractVersion": 1,
  "kind": "result",
  "exitCode": 0,
  "signal": null,
  "stdout": "deployment successfully rolled out\n",
  "stderr": "",
  "stdoutBytes": 37,
  "stderrBytes": 0,
  "stdoutTruncated": false,
  "stderrTruncated": false,
  "durationMs": 842.4
}
```

The public facade removes `contract`, `contractVersion`, and `kind`, then
returns a deeply read-only Process result.

At least one of `exitCode` or `signal` is non-null. `stdoutBytes` and
`stderrBytes` describe bytes observed before the configured capture boundary;
the handler continues draining pipes after truncation so a verbose child
cannot deadlock.

## Exit failure

An exit code not present in `acceptExitCodes` is a failed capability call:

```json
{
  "kind": "service_rejected",
  "code": "WOML_PROCESS_EXIT_CODE",
  "message": "The supervised process exited with code 1.",
  "retryable": false,
  "ambiguous": false,
  "details": {
    "exitCode": 1,
    "stdoutBytes": 120,
    "stderrBytes": 84,
    "stdoutTruncated": false,
    "stderrTruncated": false
  }
}
```

Raw output is not included in CapabilityFailure because that same safe failure
is persisted in durable events. Process v1 does not expose failed-command
output to a catching script. The later generalized streaming-output contract
may expose redacted ephemeral output without adding it to durable events.

Authors who intentionally accept a tool's nonzero status must declare that
code in `acceptExitCodes`; the returned result then includes its captured
output.

## Effect and identity

Every Process v1 call is `UnsafeWrite`. WOML cannot infer the behavior of an
arbitrary executable from its name or arguments.

One automatic Process call is allowed per step. Multiple calls require stable
names. Stable operation keys use the unchanged Capability Call v1 derivation.
Process v1 has no provider idempotency key.

## Durable metadata

Allowed start metadata:

- executable policy alias;
- argument count;
- working-directory policy identifier;
- timeout;
- output modes and byte limit.

Allowed terminal metadata:

- outcome (`succeeded`, `failed`, `timed_out`, `cancelled`, or `interrupted`);
- exit code or signal;
- duration;
- stdout and stderr byte counts;
- stdout and stderr truncation flags.

Forbidden durable data:

- argument contents;
- environment values;
- stdin;
- stdout and stderr content;
- resolved executable path when the active policy marks it sensitive;
- ambient process environment;
- secret values.

## Permission contract

Process execution is disabled without Process Policy v1. The workflow cannot
grant itself permission.

Process Policy v1 is a separate contract rather than a Runtime Configuration
version. This lets security policy evolve, be signed, and be deployed
independently from ports, logging, storage, and other operational settings.

The policy freezes:

- whether foreground and background execution are independently allowed;
- a canonical project root;
- executable alias-to-path mappings;
- allowed working-directory roots;
- inherited environment names;
- explicit environment names;
- timeout, output, and concurrency ceilings.

The policy schema is `docs/schemas/process-policy.v1.schema.json`.

### Local foreground grant

SP2 may expose the reviewed shorthand:

```bash
woml run checks.woml --allow-process bun,git
```

This is an ephemeral foreground-only policy overlay. The CLI resolves each
alias from the operator's startup environment before activation, prints the
canonical paths it intends to grant, and restricts working directories to the
workflow project root. It never grants a shell. Unknown aliases fail preflight.

`--allow-process` is rejected with `--background`. A background runtime must
load a durable Process Policy v1 document with `modes.background: true`; a
workflow file cannot create or widen that policy.

## Cancellation and timeout

The handler supports cancellation. Cancellation and timeout target the complete
contained process tree, not only the immediate child.

- Linux and macOS use an isolated process group and bounded graceful-to-forceful
  termination.
- Windows uses a Job Object configured to terminate contained processes when
  the job closes.

`timed_out` and `cancelled` remain distinct Capability Failure kinds. Both are
ambiguous because the process may already have changed external state before
termination.

## Recovery

Rust records `operation_started` before spawn and a terminal operation event
before releasing a result to Bun.

After recovery, an unmatched Process `operation_started` is:

- `interrupted`;
- ambiguous;
- not retryable automatically;
- never replayed by WOML.

An operator may inspect the run and start a reviewed new run. Process v1 does
not provide exactly-once command execution.

## Failure catalog

| Code | Kind | Retryable | Ambiguous | Meaning | Corrective action |
|---|---|---:|---:|---|---|
| `WOML_PROCESS_DISABLED` | `service_rejected` | No | No | No Process policy enabled the capability | Add an operator-owned policy or an explicit foreground grant |
| `WOML_PROCESS_MODE_DENIED` | `service_rejected` | No | No | Policy does not allow foreground or background mode | Enable that mode in Process Policy v1 |
| `WOML_PROCESS_EXECUTABLE_DENIED` | `service_rejected` | No | No | Alias is absent from policy | Add a reviewed alias-to-path mapping |
| `WOML_PROCESS_EXECUTABLE_NOT_FOUND` | `service_rejected` | No | No | Approved path does not exist | Install the tool or correct its policy path |
| `WOML_PROCESS_EXECUTABLE_CHANGED` | `service_rejected` | No | No | Approved executable failed identity validation | Re-run preflight and review the replacement binary |
| `WOML_PROCESS_CWD_DENIED` | `service_rejected` | No | No | Working directory escapes approved roots | Use a project-relative approved directory |
| `WOML_PROCESS_CWD_NOT_FOUND` | `service_rejected` | No | No | Working directory does not exist | Create or correct the directory |
| `WOML_PROCESS_ENV_DENIED` | `service_rejected` | No | No | Request supplies a forbidden environment name | Add the name to policy or remove it from the request |
| `WOML_PROCESS_INPUT_INVALID` | `invalid_input` | No | No | Request violates Process v1 | Correct the field named by the diagnostic |
| `WOML_PROCESS_CONCURRENCY_LIMIT` | `service_rejected` | Yes | No | Policy concurrency is exhausted | Wait, reduce parallelism, or raise the reviewed limit |
| `WOML_PROCESS_SPAWN_FAILED` | `service_rejected` | Yes | No | OS rejected spawn before child execution was observed | Check permissions and host resources, then retry explicitly |
| `WOML_PROCESS_CONTAINMENT_FAILED` | `handler_crashed` | No | Yes | The spawned process could not be attached to the required OS containment boundary | Inspect host process/job permissions before starting a reviewed new run |
| `WOML_PROCESS_PIPE_DRAIN_TIMED_OUT` | `handler_crashed` | No | Yes | Output pipes remained open after bounded process-tree termination | Inspect descendant behavior and start a reviewed new run |
| `WOML_PROCESS_EXIT_CODE` | `service_rejected` | No | No | Process returned a non-accepted exit code | Fix the command or explicitly accept the documented code |
| `WOML_PROCESS_SIGNALLED` | `service_rejected` | No | Yes | Process terminated by signal or platform equivalent | Inspect the host and start a reviewed new run |
| `WOML_PROCESS_TIMED_OUT` | `timed_out` | No | Yes | Runtime terminated the process tree after timeout | Inspect partial effects before starting a new run |
| `WOML_PROCESS_CANCELLED` | `cancelled` | No | Yes | Runtime cancelled the process tree | Inspect partial effects before starting a new run |
| `WOML_PROCESS_INTERRUPTED` | `interrupted` | No | Yes | Durable start has no terminal outcome after recovery | Inspect the run; never assume it is safe to replay |

Generic Capability Call failures keep their existing codes: input and result
size limits use `WOML_CAPABILITY_INPUT_TOO_LARGE` and
`WOML_CAPABILITY_RESULT_TOO_LARGE`; handler failure uses
`WOML_CAPABILITY_HANDLER_CRASHED`; script-host loss uses
`WOML_SCRIPT_HOST_CRASHED`. They retain the existing retry and ambiguity
semantics rather than creating Process-specific duplicates.

Output truncation is a successful-result condition expressed by
`stdoutTruncated` and `stderrTruncated`, not a failure. Authors can request a
larger bounded limit when policy allows it.

## Environment and output redaction

The Process authority starts every child with an empty environment. It then
adds only values named by `environment.inherit` and explicit `env` values whose
names are permitted by `environment.allow`. WOML's other secrets are not
implicitly exposed to the child.

On Windows, the cleared environment additionally receives `SystemRoot`,
derived from `GetSystemWindowsDirectoryW` rather than the parent environment.
Windows runtime initialization requires this OS bootstrap value. Application
variables, including `PATH` and user-profile directories, remain subject to
the explicit policy lists. Approved inherited or request values are applied
after the bootstrap value and retain their existing precedence.

Operator-inherited environment values are treated as sensitive by the Rust
authority. Resolved WOML secret values are known only inside the isolated Bun
worker, so that boundary replaces their exact occurrences with `[REDACTED]`
before captured stdout or stderr becomes script-visible. It also masks a
truncated stream suffix when that suffix matches the beginning of a known
secret. Rust redaction includes a bounded lookahead at the capture boundary so
truncation cannot reveal the beginning of an inherited value. Secret
provenance intentionally does not cross Process v1, and ordinary explicit
`env` values are not guessed to be secrets. Environment values, arguments, and
captured output remain absent from durable events and diagnostics.

No output filter can reliably recognize every transformation of a secret. A
child can encode, hash, split, encrypt, compress, or otherwise transform a
value before printing it. Process v1 therefore documents redaction as a
defense-in-depth boundary, not permission to print secrets. Workflows should
avoid passing secrets to tools that may echo or transform their environment.

## Terminal activity

Human terminal output renders each managed Process call beneath its owning
step. It may show only the approved executable alias, outcome, safe exit code
or signal, duration, observed stdout/stderr byte counts, truncation state, and
the configured output ceiling while running. A running duration is calculated
ephemerally by the terminal renderer and is never added to durable events. It
never shows arguments, environment values, resolved command paths, or raw
captured output as Process activity. JSON presentation remains ANSI-free and
uses the same safe durable metadata.

## Recovery decision table

| Observed durable state | Runtime decision | Automatic replay |
|---|---|---|
| No `operation_started` | No process effect was admitted | Not applicable |
| Start and success | Return the recorded terminal outcome | No |
| Start and failure/cancellation | Return the recorded terminal failure | No |
| Start without terminal event | Mark interrupted and ambiguous | Never |

## Platform containment expectations

| Platform | Containment boundary | Timeout/cancellation | Host-crash expectation |
|---|---|---|---|
| Linux | Dedicated process group, with parent-death support where proven | Graceful group termination, then forced group kill | Descendants should die when the boundary closes; recovery remains ambiguous |
| macOS | Dedicated process group | Graceful group termination, then forced group kill | Audit possible orphans because parent-death behavior is weaker |
| Windows | Kill-on-close Job Object | Terminate the complete Job Object | Closing the job must terminate contained descendants |

No platform is claimed conformant until its packaged release passes the SP4
and SP8 descendant-cleanup tests.

## Deferred features

- shell strings;
- PTYs and interactive stdin;
- detached children;
- process pipelines;
- generalized live log streaming;
- automatic artifact output;
- remote execution targets;
- trusted declarations that a command is read-only or idempotent.
