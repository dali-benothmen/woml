# WOML Diagnostics and Repair

WOML errors carry a stable code, source location when authoring is involved, a message, and often a repair hint. Fix the contract violation; do not suppress validation or edit compiled JSON/native state directly.

## Authoring failures

| Symptom/code | Likely cause | Repair |
| --- | --- | --- |
| `WOML_UNKNOWN_ELEMENT` | Misspelled, misplaced, unfinished, or invented tag | Check [tags.md](tags.md); use only a released element in an allowed parent. |
| Unknown attribute diagnostic | Attribute belongs on another tag or has the wrong spelling | Check the exact element attribute table; do not add aliases. |
| Duplicate identity diagnostic | IDs collide across executable/control structures | Choose stable unique lower-camel IDs; preserve existing IDs when repairing deployed workflows. |
| Invalid reference/visibility diagnostic | Forward, sibling, unselected-route, or unjoined-branch read | Move the consumer, merge through `<result>`, join the branch, or return a dominating value. |
| `WOML_REFERENCE_NOT_AVAILABLE` | A statically valid nested property is missing at runtime | Make every producing route return the required shape or validate data before referencing it. |
| Invalid script result | Returned value is not JSON-compatible or exceeds limits | Return compact JSON; remove clients, undefined, circular data, BigInt, or oversized bodies. |
| Invalid condition/switch value | `<when>` is not boolean or `<switch>` value is not string | Normalize the value in an earlier script step; WOML does not coerce. |

Always use the reported line/column. Do not rewrite unrelated parts of the workflow when one local source error is enough.

## Scripts and workers

| Code | Meaning and repair |
| --- | --- |
| `WOML_SCRIPT_THROWN` | The authored script threw. Inspect its safe message/stack and correct code or inputs. It is the definitive failure eligible for configured retry. |
| `WOML_SCRIPT_TIMEOUT` | Step exceeded its execution policy. Reduce work, use a managed timeout appropriately, or change reviewed workflow policy. Do not blindly increase limits. |
| `WOML_SCRIPT_NON_JSON_RESULT` | Return value cannot cross the JSON boundary. Return supported JSON. |
| `WOML_SCRIPT_CONTEXT_TOO_LARGE` | Too much durable context is being passed. Return smaller step outputs and move large data to storage. |
| `WOML_SCRIPT_RESULT_TOO_LARGE` | One result exceeds its boundary. Store the large object and return its reference. |
| `WOML_SCRIPT_WORKER_CRASHED` / `WOML_SCRIPT_HOST_CRASHED` | Isolated worker/host stopped. Treat started external effects as potentially ambiguous; inspect/recover rather than replaying blindly. |

## Secrets

| Code | Repair |
| --- | --- |
| `WOML_SECRET_NOT_FOUND` | Configure the named secret with `woml secrets set NAME` or the selected production secret provider. |
| `WOML_SECRET_NAME_INVALID` | Use uppercase symbolic names such as `API_TOKEN`. |
| `WOML_SECRET_PROVIDER_READ_ONLY` | Configure the mounted/environment provider externally instead of calling `secrets set`. |
| Secret source conflict | Ensure a name comes from one reviewed provider/source. |

Never ask the user to paste a secret into WOML source or logs.

## Modules and reusable definitions

| Symptom/code | Repair |
| --- | --- |
| `WOML_MODULE_UNUSED` | Informational warning: remove the unused import or call it intentionally. |
| `WOML_MODULE_SERVICE_UNKNOWN` | Misspelled/undeclared `services.<alias>` | Match `<module name="...">` exactly and use a named export. |
| Module digest/cache mismatch | Stored artifact and requested immutable definition disagree | Stop, preserve state, rebuild/redeploy coherently; do not bypass digest checks. |
| Invalid reusable props | Missing required prop, undeclared prop, or wrong secret binding | Match declared kebab-case props and use exact secret references for secret props. |
| Unknown provider artifact | Imported provider bundle/definition is absent or mismatched | Check the import graph and run `woml check` on the complete workflow deployment. |

## Triggers

| Code | Repair |
| --- | --- |
| `WOML_TRIGGER_SCHEMA_INVALID` | Payload does not satisfy inline schema. Correct caller data or deliberately revise schema. No run was created. |
| `WOML_TRIGGER_UNAUTHORIZED` | Missing/incorrect bearer credential. Send the configured token without printing it. |
| `WOML_TRIGGER_METHOD_NOT_ALLOWED` | Wrong HTTP method. Use the generated curl/declared POST route. |
| `WOML_TRIGGER_NOT_FOUND` | Wrong path/event or workflow not loaded. Verify readiness output and active inputs. |
| `WOML_TRIGGER_IDEMPOTENCY_CONFLICT` | Same external event identity was reused with different content. Generate stable unique publisher IDs and never mutate an existing identity. |
| `WOML_TRIGGER_UNAVAILABLE` | Runtime/store/provider authority could not admit the event. Inspect the underlying actionable provider/store diagnostic. |
| Manual trigger TTY/selection errors | Manual automation lacks an interactive terminal or several manual IDs exist | Use an interactive `woml run`, select `--trigger`, or use `woml test` for CI. |

## Services

Managed service errors expose `code`, `service`, `operation`, `retryable`, and `ambiguous`.

- `retryable: true` means a later attempt may be appropriate; it does not override step retry policy.
- `ambiguous: true` means the external effect may already have occurred. Do not repeat it without provider idempotency or reconciliation.
- Database statement failures: inspect parameterized SQL and driver-specific placeholder syntax.
- Storage not-found/conflict/path-unsafe: verify the logical key and `ifVersion`/overwrite contract; never convert it to a raw filesystem path.
- Cache miss is a successful normal result, not an error.
- `WOML_STATE_CONFLICT`: another writer changed the version. Read again and make a new named decision.
- State quota/corruption/unavailable errors require operational review; never edit internal tables to silence them.

## Workflow calls

| Code | Repair |
| --- | --- |
| `WOML_WORKFLOW_TARGET_NOT_FOUND` | Load the exact child ID in the same runtime or another process sharing the state file. |
| `WOML_WORKFLOW_TARGET_AMBIGUOUS` | More than one live owner registered the target; stop the duplicate owner. |
| `WOML_WORKFLOW_CALL_CYCLE` | Parent/child lineage would recurse; redesign the workflow graph. |
| `WOML_WORKFLOW_CALL_WAIT_UNSUPPORTED` | Synchronous `.call()` targeted approval/long-wait behavior; use `.start()` if the parent need not wait. |
| `WOML_WORKFLOW_CALL_TIMED_OUT` | Parent stopped waiting; inspect the independently durable child before deciding what to do. |
| `WOML_WORKFLOW_CALL_FAILED` | Inspect the child run ID for its actual failure. |
| Idempotency conflict | Reused operation name with a different target/payload | Give each logical call a stable unique name and keep input stable across retry. |

## Approvals and notifications

| Code | Repair |
| --- | --- |
| `WOML_NOTIFICATION_DELIVERY_FAILED` | Every configured approval delivery failed. Check credentials, destination membership/permissions, and provider availability. |
| Notification delivery ambiguous | Provider may have accepted the message before interruption. Reconcile rather than manufacturing another logical delivery. |
| Invalid/expired approval token | Use the current capability from the delivered message/runtime; never persist a token in source. |
| Approval decision conflict | A first decision already won. Do not attempt to reverse it. |
| `WOML_APPROVAL_TIMEOUT` | `on-timeout="fail"` deadline expired. Use reject only when that business behavior is intended. |

For Telegram/Discord/WhatsApp, run the corresponding `doctor` command. Slack has no public doctor command; verify Socket Mode, event subscriptions, OAuth scopes, bot membership, and destination names/IDs.

## Runtime and state operations

- If `woml run --background` fails, read the printed runtime log path and fix the actionable startup error.
- A stale/unsafe runtime descriptor must not be manually trusted or followed. Stop stale processes and let WOML recreate safe metadata.
- Use the same `--state` path for `run`, `inspect`, `list`, `get`, `cancel`, logs, backup, restore, and prune when targeting one deployment.
- Start retention with `woml prune --dry-run`.
- Restore to a separate path first when investigating a production backup; `--replace` needs explicit authorization.

## Agent repair procedure

1. Preserve the exact error code, message, and source location.
2. Identify whether it is authoring, configuration, provider setup, runtime state, or an external system failure.
3. Read only the relevant skill reference and repository documentation.
4. Make the narrowest safe correction.
5. Run `woml check` again.
6. Re-run or activate only when the user authorized execution and external effects are safe.
7. Report what was validated and what still requires credentials, provider configuration, network access, or user action.
