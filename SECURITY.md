# Security Policy

## Supported versions

Security fixes are provided for the latest published WOML release. Before the
first public package is published, the source on the default branch is the only
supported development line.

## Report a vulnerability privately

Do not open a public issue for a suspected vulnerability. Use GitHub's
**Security** tab and select **Report a vulnerability** to create a private
report for the maintainers.

Include only what is necessary to reproduce and assess the problem:

- the WOML and operating-system versions;
- the affected command, trigger, provider, or service;
- expected and observed behavior;
- a minimal reproduction with fake credentials and sanitized payloads; and
- the likely impact and any known workaround.

Never include real tokens, `.woml/` state, production databases, provider
signing secrets, approval URLs, or unredacted customer data. If a large or
sensitive artifact is necessary, wait for a maintainer to provide a private
transfer method.

## Security boundaries

WOML treats source files and imported modules as trusted application code. The
runtime isolates authored JavaScript execution and supervises capabilities, but
it is not a sandbox for hostile workflow authors. Operators remain responsible
for host isolation, file permissions, outbound-network policy, TLS, secret
provisioning, provider permissions, and backup protection.

See [WOML data security](docs/woml-data-security.md) and the
[production deployment checklist](docs/woml-production-deployment.md) for the
supported operational boundary.

Infrastructure Policies can further restrict protected effects and require
engine-owned approval, artifact, or lock evidence. They do not replace Process
Policy, Artifact Policy, host isolation, cloud IAM, Kubernetes admission,
database permissions, or operating-system controls. Static policy explanation
is advisory; only the Rust pre-effect authority makes the runtime decision. See
[Infrastructure Policies](docs/infrastructure-policies-v1.md).

## Disclosure

Please allow maintainers time to confirm the issue and prepare a coordinated
fix before publishing details. The eventual advisory will credit reporters who
want attribution and will avoid exposing secrets or unsafe reproduction data.
# Project and remote-execution boundary

Treat `workflow.json`, policy files, reviewer directories, target registries,
and trust files as reviewed configuration. They may contain symbolic secret
names and certificate references, but must not contain private keys or secret
values. Remote targets are authenticated and policy-intersected; they are not a
generic shell. Work with an unknown remote outcome becomes **ambiguous** and is
blocked from ordinary retry until an operator inspects and reconciles it. See
the [combined operator runbook](docs/combined-developer-experience-operator-runbook.md).
