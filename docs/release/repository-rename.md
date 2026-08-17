# Repository Rename Checklist

The public WOML source identity is `dali-benothmen/woml`. Source manifests,
documentation links, extension metadata, and release metadata already use that
identity.

The local Git remote must not be changed until the GitHub repository itself has
been renamed.

After the owner renames the repository in GitHub settings:

```bash
git remote set-url origin https://github.com/dali-benothmen/woml.git
git remote -v
git ls-remote origin HEAD
```

Both fetch and push URLs must display the WOML repository. Existing branches,
tags, pull requests, issues, and Git history remain attached to the renamed
GitHub repository. Do not create a second unrelated repository for the same
release history.

Before V1R9 publication, verify that npm trusted publishing, GitHub environment
protection, issue links, and the VS Code Marketplace repository link all name
the renamed repository.
