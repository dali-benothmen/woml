# WOML for Visual Studio Code

Native-looking syntax highlighting for Workflow Orchestration Markup Language
files.

## What is included

- `.woml` language registration.
- HTML-style tags, attributes, comments, and strings using the active editor
  theme.
- Native JavaScript highlighting inside `<script>`.
- Special built-in-variable highlighting for `context`, `services`, `secrets`,
  `props`, `lifecycle`, and `attempt`.
- WOML `{{...}}` reference-expression highlighting.
- Workflow and step snippets.

The extension contributes no color theme. Its TextMate scopes deliberately use
the active theme's existing HTML, JavaScript, and built-in-variable colors.

## Check the extension

From the repository root:

```bash
cd woml-vscode
bun run check
```

## Build and install a local VSIX

```bash
cd woml-vscode
bun run package
code --install-extension woml-language-0.1.1.vsix
```

Reload Visual Studio Code after installation and open any `.woml` file.

## Inspect a token while developing

Run **Developer: Inspect Editor Tokens and Scopes** from the VS Code command
palette, then click the token. WOML runtime globals should include the
`support.variable.woml` scope.
