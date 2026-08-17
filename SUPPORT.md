# WOML Support

Use [GitHub Discussions](https://github.com/dali-benothmen/woml/discussions)
for questions, workflow design help, and ideas. Use
[GitHub Issues](https://github.com/dali-benothmen/woml/issues) for reproducible
bugs and concrete feature requests.

Before asking for help:

1. run `woml --version`;
2. run `woml check <workflow>`;
3. retry with `--verbose` when the command supports it;
4. check the [documentation map](docs/README.md) and provider doctor command;
5. reduce the problem to the smallest safe workflow.

A useful bug report includes the WOML version, operating system/architecture,
command, stable error code, source location, expected behavior, actual behavior,
and minimal reproduction.

Do not post secret values, approval tokens/URLs, `.woml/` state, databases,
private payloads, provider event bodies, or unredacted logs. Replace identifiers
and data with safe samples while preserving the structure that reproduces the
problem.

Report security vulnerabilities privately according to
[SECURITY.md](SECURITY.md), never through an issue or discussion.
