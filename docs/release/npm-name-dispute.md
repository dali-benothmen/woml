To:      support@npmjs.com
Subject: Name dispute: claim of `woml` (project identity, not a typosquat of `yaml`)

Hello npm Support,

I am the maintainer of the WOML workflow automation project (formerly
published on npm as `cronflow`, versions 0.1.0 through 0.11.6, now
deprecated under that name following the rename to `woml`). I am
trying to publish the v1.0.0 release of `woml` as an unscoped npm
package, but npm's automated anti-squatting check is blocking the
publish with the error:

    403 Forbidden - PUT https://registry.npmjs.org/woml -
    Package name too similar to existing packages toml, xml,
    mjml, yaml, ol, rome; try renaming your package to
    '@mohamedalibenothmen/woml' and publishing with
    'npm publish --access=public' instead

I am not typosquatting any of those packages. `woml` is the official
identity of the project: "Workflow Orchestration Markup Language".
It is the public-facing name chosen for the v1.0.0 rewrite of the
engine and has been the canonical name on the project's GitHub
repository since the rename from `cronflow`. There is continuous git
history across both names — this is not a new typosquat project.

Evidence of legitimate claim:

  * GitHub repo (active since 2025-07-21):
    https://github.com/dali-benothmen/woml
  * Apache-2.0 LICENSE in the repo:
    https://github.com/dali-benothmen/woml/blob/master/LICENSE
  * Project documentation under the name WOML:
    https://github.com/dali-benothmen/woml/tree/master/docs
  * Existing npm presence under the related org scope `@woml-org`
    (we own that scope as the `woml-org` org owner). The matching
    native-binary packages have been published under it:
      @woml-org/cli-linux-x64-gnu@1.0.0   (published)
      @woml-org/cli-darwin-x64@1.0.0     (pending publish)
      @woml-org/cli-darwin-arm64@1.0.0   (pending publish)
      @woml-org/cli-win32-x64-msvc@1.0.0 (pending publish)
      @woml-org/cli-win32-arm64-msvc@1.0.0 (pending publish)
      @woml-org/cli-linux-x64-musl@1.0.0 (pending publish)
      @woml-org/cli-linux-arm64-gnu@1.0.0 (pending publish)
      @woml-org/cli-linux-arm64-musl@1.0.0 (pending publish)
  * The previous npm package `cronflow` was maintained by this same
    npm account at https://www.npmjs.com/package/cronflow — proof of
    lineage and continuous ownership of the project's npm presence.
  * Project README, CLI, and language reference are all published
    under the name `woml` in the GitHub repo and the published
    documentation.

Could you whitelist the name `woml` so we can publish our v1.0.0
release under it? The user-facing install command is `npm i -g woml`
and we would prefer to keep that unscoped. We are willing to rename
to `@woml-org/woml` if the whitelist cannot be granted, but the
unscoped name is our project's official identity.

Thank you for your time.

Mohamed Ali Ben Othmen
npm username:    mohamedalibenothmen
npm email:       mohamedalibenothmen1@gmail.com
GitHub:          https://github.com/dali-benothmen/woml
