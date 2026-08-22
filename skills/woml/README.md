# WOML Skill for AI Agents

Give an AI coding agent the product knowledge it needs to create valid, readable WOML workflow automation from a plain-language request.

The skill teaches agents how to:

- choose triggers, steps, and control flow;
- write embedded JavaScript with `context`, `attempt`, `services`, and `secrets`;
- use retries, approvals, lifecycle hooks, modules, and workflow calls correctly;
- select from every released WOML tag, built-in service, provider surface, and CLI command;
- protect credentials and avoid invented WOML syntax; and
- validate generated workflows with the real WOML CLI.

## Install

### Project installation

Copy this complete folder into your agent's project-level skills directory. For Codex and other clients that discover the Agent Skills convention:

```bash
mkdir -p .agents/skills
cp -R /path/to/woml/skills/woml .agents/skills/woml
```

Commit `.agents/skills/woml` when everyone working on the project should use the same WOML guidance.

### Personal installation

To make the skill available across your Codex projects:

```bash
mkdir -p "${CODEX_HOME:-$HOME/.codex}/skills"
cp -R /path/to/woml/skills/woml "${CODEX_HOME:-$HOME/.codex}/skills/woml"
```

For another Agent Skills-compatible tool, copy `skills/woml` into that tool's documented skills directory. Keep the `woml` directory intact so `SKILL.md` can find its `references/` files.

Restart or reload the agent after installation if it does not immediately discover new skills.

## Use it

Ask naturally:

```text
Build a WOML workflow that receives an order webhook, validates the payload,
checks inventory and fraud risk concurrently, and returns one decision.
```

Or invoke it explicitly in clients that support named skill invocation:

```text
$woml Create a daily report workflow and save it as daily-report.woml.
```

The agent should create the workflow, run `woml check` when the CLI is available, list required secrets, and give you the exact command needed to activate and trigger it.

## Contents

```text
woml/
├── README.md
├── SKILL.md
└── references/
    ├── cli.md
    ├── context-and-scripts.md
    ├── diagnostics.md
    ├── modules.md
    ├── patterns.md
    ├── providers.md
    ├── services.md
    └── tags.md
```

- [`SKILL.md`](./SKILL.md) defines when and how an agent should build WOML.
- [`references/tags.md`](./references/tags.md) contains every released tag, attribute, placement, and output rule.
- [`references/context-and-scripts.md`](./references/context-and-scripts.md) documents JavaScript and declarative bindings.
- [`references/services.md`](./references/services.md) covers every built-in service and its reliability boundary.
- [`references/modules.md`](./references/modules.md) covers JS/TS imports, custom steps, and custom notification providers.
- [`references/providers.md`](./references/providers.md) covers communication triggers, notifications, messaging, setup, and secrets.
- [`references/cli.md`](./references/cli.md) documents all public commands and important flags.
- [`references/diagnostics.md`](./references/diagnostics.md) maps common failures to safe repairs.
- [`references/patterns.md`](./references/patterns.md) explains how to select reliable automation structures.

The skill intentionally follows released WOML behavior. Update it alongside public language or CLI changes so agents never learn unfinished syntax.
