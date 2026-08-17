# WOML Script-Host Protocol v2

Status: frozen for the parallel profile

The normative machine-readable contract is
`docs/schemas/script-host-protocol.v2.schema.json`. Protocol v1 remains
immutable and supported.

Protocol v2 keeps the long-lived Bun host, one isolated Worker per invocation,
Content-Length-framed UTF-8 JSON, asynchronous multiplexing, and
`invocationId` correlation. Rust may have multiple execute messages in flight;
completed responses may arrive in any order.

The only new request shape is:

```json
{
  "protocol": "woml.script-host",
  "protocolVersion": 2,
  "messageType": "cancel",
  "invocationId": "inv_load_soil_01",
  "reason": "parallel_fail_fast"
}
```

Every accepted execute still has exactly one terminal `completed` response. If
cancellation wins, the failure is `invocation_cancelled` with code
`WOML_SCRIPT_CANCELLED`. If execution wins, its real outcome remains final and
a late cancel is a no-op. Unknown and already-terminal invocation IDs are also
safe no-ops. Cancellation terminates only the addressed Worker, does not imply
rollback, and is distinct from Worker or host crashes.

The reviewed fixtures live in
`woml-cli/tests/fixtures/script-host-v2/`.
