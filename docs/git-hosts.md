# Git hosts

GitHub and Azure DevOps, detected from the repo URL rather than configured.
Cloning, and pushing the work branch (the agent's own `git push`, or the push
button in the window's git pane), happen *inside* the sandbox, so the host
never holds the credential:

```sh
hurad new --repo 'https://dev.azure.com/org/project/_git/repo' \
        --task "..." --provider azure-devops --provider claude
```

`--provider` names a `[credentials.NAME]` table in the config file, and the
sandbox never sees the value behind it. The session gets a random placeholder in
`AZURE_DEVOPS_PAT` or `GITHUB_TOKEN`, and the sandbox runtime's proxy writes the
real token over it in requests to that forge's hosts (`dev.azure.com`;
`github.com` and `api.github.com`) and to nothing else. The clone writes the
header into the repository's `http.extraHeader`, so a later `git push` needs no
special casing; what is written there is the placeholder, which means nothing
outside that sandbox.

```toml
[credentials.azure-devops]
kind = "azure-devops-pat"                 # Code (Read & Write)
command = "tr -d '\r\n' < /home/me/.config/hura/tokens/azure-devops"

[credentials.github]
kind = "github"
command = "tr -d '\r\n' < /home/me/.config/hura/tokens/github"
```

The header is `Authorization: Bearer $GITHUB_TOKEN` on GitHub and
`Authorization: Basic $AZURE_DEVOPS_PAT` on Azure DevOps. A PAT is a Basic
*password* with an empty user, so what the runtime stores for it is
`base64(":" + PAT)`: a `command` prints the PAT as it is and hura wraps it to
store the encoded form, while a `ref` has to name a secret already holding that
form. Sent as a bearer token instead, a PAT gets a redirect to a sign-in page
rather than a 401. [configuration.md](configuration.md#credentials) has the
rest of how credentials are configured.

**An Azure DevOps PAT is scoped to one organisation.** One minted for one org
gets a 401 from another (measured), so mint one per org, give each its own
`[credentials.NAME]` table, and tick the right one per session. A session takes
one credential of each kind, so one session reaches one organisation.

Pull requests are the agent's to open (`gh` on GitHub, an MCP server or the
REST API on Azure DevOps, both of which the `feature-work` policy grants: the
git paths, `_apis` on `dev.azure.com`, and `api.github.com`), or yours, from
the host's own web UI. `readonly-explore` reaches neither `git-receive-pack` nor
`_apis`, so a session under it can read a repository and provably cannot push
to it.

The image the agent runs in, and the settings baked into it, are
[sandbox-image.md](sandbox-image.md).

---

[← Documentation](README.md) · [README](../README.md)
