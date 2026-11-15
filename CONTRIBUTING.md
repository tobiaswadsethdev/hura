# Contributing

Thanks for looking. `hura` is early and small, which makes it a good size to
contribute to: the whole thing is two crates, the test suite runs in under a
second, and almost all of it can be worked on without a sandbox runtime anywhere
near your machine.

Issues, questions and pull requests are all welcome. If you are unsure whether
something is wanted, open an issue first -- that is cheaper for both of us than
a branch that turns out to be aimed somewhere else.

## What you need

To **build and test**: Rust 1.89 or newer, and nothing else. The suite is
hermetic on purpose: no sandbox runtime, no Docker, no network.

To **run it against real sandboxes**: everything in
[docs/install.md](docs/install.md): Linux with systemd and KVM, Docker Sandboxes
signed in, Docker 29.x, tmux. That is worth setting up if you are changing how
sessions are created, seeded or published; it is not worth setting up to fix the
diff pane's wrapping.

## The loop

```sh
cargo build
cargo test --workspace                                    # 513 tests
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p hurad -- doctor                               # the CLI, from the tree
cargo run -p hurad -- serve                                # the server, from the tree
```

CI runs exactly those last three checks on the **newest stable**, so
`rustup update stable` before pushing is what makes a green local run a green
pull request: clippy gains lints with each release, and one of them can turn CI
red on code nobody touched. The sandbox runtime's contract lives in ignored tests
that need a signed-in `sbx` and KVM, and creates and removes real sandboxes named
`hura-test-*`:

```sh
cargo test -p sbx-client -- --ignored --test-threads=1
```

To run a development `hurad` beside an installed one without the two sharing a
session cache, point `XDG_CONFIG_HOME` and `XDG_STATE_HOME` somewhere of its
own. The sandbox runtime is not affected: `sbx-client` unsets those variables
for every call it makes, so the development server still finds the one daemon
and the one sign-in.

[docs/architecture.md](docs/architecture.md) is the map of the crates and
modules -- worth ten minutes before your first change.

## House style

The code has a voice, and matching it is most of what review here is about.

* **Say why, not what.** Every module starts with a `//!` block explaining what
  the module is *for* and which alternative was rejected. New modules get one;
  changed behaviour updates the existing one. If a comment could be deleted
  without losing information, delete it.
* **Tests stay hermetic.** A test that needs the sandbox runtime, Docker or a
  network goes behind `#[ignore]`. Pane classification is tested against captured specimens
  in `crates/hura-core/tests/panes/`; add a specimen rather than a mock when you are
  teaching it a new agent state.
* **No I/O on a render path.** Sandbox runtime calls are subprocess round trips
  costing hundreds of milliseconds. They belong behind `hura-core`, reached over `/rpc`;
  nothing that paints may make one.
* **Failures name their fix.** `hurad doctor` checks and error messages both say
  what to do about the problem, not just that there is one. A misspelled config
  key is named back at the user; a credential the config names but does not
  define is reported before it becomes a clone failure three steps later.
* **The isolation is the product.** A change that widens what a sandbox can
  reach needs to be visible in the policy pane and defensible in the docs. If it
  is a hole, [docs/mcp.md](docs/mcp.md) is the tone to aim for -- say plainly
  what it costs.
* `cargo fmt` decides formatting. Don't argue with it in review.

## Pull requests

* One change per pull request. A drive-by rename in the same diff as a bug fix
  makes both harder to review.
* Say **why** in the description: what was wrong, what you did about it, and how
  you know it works. The pull request template asks for exactly that.
* Add tests for behaviour you change. If the change is genuinely untestable
  without a sandbox runtime, say so in the description and describe what you ran
  by hand against a development `hurad`.
* Update the docs in the same pull request. User-visible behaviour lives in
  `docs/`, and the README links to it.
* Keep `cargo fmt` and `cargo clippy -- -D warnings` clean.

Commit messages: a short imperative summary, then a body explaining the
reasoning if the change is not obvious. `git log` here is a record of decisions
rather than a list of files touched.

## Releasing

Three things release from this repository, each with its own version and its
own tags. Up to v0.54.0 they shared one version, tagged `v...`; what they have
to agree on is the protocol (`hura_proto::VERSION`), which a client checks when
it connects, and a protocol change is the one thing that means releasing more
than one of them.

| component | tag | version in | released by |
| --- | --- | --- | --- |
| hurad | `hurad-vX.Y.Z` | `Cargo.toml`, `Cargo.lock` (and the desktop app's lockfile, for the shared crates) | `release-hurad.yml` |
| desktop app | `desktop-vX.Y.Z` | `apps/desktop/package.json`, `package-lock.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/tauri.conf.json` | `release-desktop.yml` |
| iPhone app | `ios-vX.Y.Z` | `apps/ios/Config/Version.xcconfig` | `apps/ios/Tools/testflight.sh`, from a Mac |

Cutting any of them is a dispatch of **Tag a release**
(`.github/workflows/tag.yml`) from the Actions tab: pick the component and
`patch`, `minor` or `major` -- or an exact version if the increment is not the
point. It works out the next version from that component's own files, rewrites
them, commits that as `Release <tag>`, tags it and pushes both; the tag starts
the release workflow. `dry_run` does everything except the pushing and prints
the diff, which is the cheap way to check a bump before it is permanent. For
the iPhone app the tag only moves the version; the next
`apps/ios/Tools/testflight.sh` from `main` uploads a build of it (see
[apps/ios](apps/ios/README.md#testflight)).

Bumping by hand is the reason this is a workflow. `v0.3.0` was tagged with
every file still reading `0.2.0`, and `hurad update` refuses a release whose
binary reports a different version than the tag claims -- so that release was
one no installed copy could update to. The workflow writes every file from one
number and refuses to tag if the diff touches anything else.

Doing it by hand still works, and is still the fallback if Actions is down:

```sh
# bump the component's files above, commit them, then:
git tag hurad-v0.55.0 && git push origin hurad-v0.55.0
```

The tag push from `tag.yml` starts the release workflow because it is made
with the release deploy key: GitHub does not run workflows on events raised
with the built-in `GITHUB_TOKEN`, and the key is also what lets the commit
bypass `main`'s required checks.

Release notes are generated from the component's previous tag (or the last
combined `v...` release, for the first of each), so they cover the merges in
between -- all of them, since GitHub does not filter them by path.

### hurad

`release-hurad.yml` builds a static musl binary for
`x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`, and one for
`aarch64-apple-darwin`, packs each as `hurad-v<version>-<target>.tar.gz` with
`hurad` flat at the root, and publishes them with one `SHA256SUMS` covering all
of them. Three files have to agree about that name, the tag and what is inside
the archive -- the workflow, `install.sh` and `crates/hura-core/src/update.rs`
-- and a test in `update.rs` fails if they ever stop agreeing, so a rename in
one of them is caught locally rather than by someone's broken install.

hurad's releases are the ones GitHub marks "latest". Nothing depends on that:
`install.sh` and `hurad update` both take the newest `hurad-v` (or older `v`)
tag from the release list, so a desktop release on top does not confuse them.

**Nothing at v0.54.0 or older can follow the tag's new prefix.** Those read
everything after a `v` as the version, find `hurad-v0.55.0` is not one, and
stay where they are, quietly. One `install.sh` moves a server across, and from
then on `hurad update` reads both prefixes, so going back to `v0.53.0` still
works.

The asset was `hura-<tag>-<target>.tar.gz` until v0.4.0, and carried `hura`.
Nothing installed before then can update across that rename either: `hura
update` replaces the binary it is running, and there is no longer an `hura` to
replace it with. v0.3.1 is the last release `hura update` can reach at all, so
nobody is left sitting on the v0.3.0 that no installed copy could update to.

### The desktop app

`release-desktop.yml` builds the Windows installers (`.exe` and `.msi`) and the
Mac disk image, named `hura-desktop-v<version>-<target>`, and publishes them
*without* marking the release latest. When there is an updater key it also
writes `latest.json`, and once the release is up commits it to
`site/desktop/latest.json` on `main`: the installed app reads it from
`https://gethura.app/desktop/latest.json`, falling back to the same file on
raw.githubusercontent.com. That needs the site to redeploy on a push to `main`.

Windows installs from v0.54.0 and before read
`/releases/latest/download/latest.json` instead, which is now a hurad release.
So the desktop workflow also uploads its `latest.json` onto the newest hurad
release, and the hurad workflow carries `site/desktop/latest.json` into each
new one. The first update such an install takes moves it onto the new address.

### The signing keys

Two, for two different jobs, and both optional in the sense that a release
without them still builds:

| secret | what it is for | without it |
| --- | --- | --- |
| `WINDOWS_CERTIFICATE`, `WINDOWS_CERTIFICATE_PASSWORD` | code-signing the installer, so SmartScreen does not warn about it | the installer is unsigned; the checksums are still the integrity story |
| `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | signing `latest.json`, which is what an installed window checks before replacing itself | no `latest.json` is published and the window never offers to update |

The updater key is the one to be careful with. Its public half is compiled into
every window ever shipped (`plugins.updater.pubkey` in `tauri.conf.json`), and
a window will not install an update signed by anything else -- so **losing the
private key means no already-installed window can ever be updated again**,
whatever is published. Changing it has the same effect on every copy already
out there. Back it up somewhere that is not this repository.

`createUpdaterArtifacts` makes `tauri build` *fail* without the key rather than
skip the signing, so `release-desktop.yml` turns that option back off when the secrets
are absent. That is what lets a fork cut a release at all.

Both halves of each pair have to be set. The key is password-protected, so a
repository with `TAURI_SIGNING_PRIVATE_KEY` and no
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` is a repository where the build tries to
sign and cannot -- which is why the workflow tests for both and treats one
without the other as no key at all.

## Reporting bugs

The single most useful thing to include is `hurad doctor` output -- it captures
the versions and half the environment problems at once. The issue templates ask
for that, plus what you expected and what happened instead.

Security problems are different: please don't open a public issue. See
[SECURITY.md](SECURITY.md).

## Licence

By contributing you agree that your contributions are licensed under the
[Apache License 2.0](LICENSE), the same terms as the rest of the project.
