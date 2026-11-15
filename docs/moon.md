# moon

Every check in the repository is a [moon](https://moonrepo.dev) task, so there
is one way to run any of them and CI runs only the ones a change can affect.
moon decides *what* runs and *when*; cargo, npm, swift and xcodebuild still do
the work, and every task is a command you could type yourself.

## Installing it

The version is pinned in `.prototools`. With [proto](https://moonrepo.dev/proto):

```sh
proto use          # installs what .prototools asks for
```

or install moon alone (`bash <(curl -fsSL https://moonrepo.dev/install/moon.sh)`)
at that version. CI gets it from `moonrepo/setup-toolchain`, which reads the same
file.

## Running things

A target is `project:task`; `:task` is that task in every project that has it.

```sh
moon run hurad:test                 # one crate's tests
moon run :fmt :clippy :test         # the Rust loop, across the workspace
moon run desktop:dev                # the desktop app against a real server
moon run relay:test
moon run ios:test                   # on a Mac, against a simulator
moon ci                             # what CI would run for this branch's changes
moon query tasks                    # every task, as JSON
moon task hura-core:test            # one task: its command, inputs, what it inherits
```

A task whose inputs have not changed since it last passed is replayed from
moon's cache (`.moon/cache`, ignored by git) rather than run again.

| project | path | tasks |
| --- | --- | --- |
| `sbx-client`, `hura-core`, `hura-proto`, `hura-client` | `crates/` | `fmt`, `clippy`, `test`; and `check-windows` for the three the desktop app links |
| `hurad` | `hurad/daemon` | `fmt`, `clippy`, `test`, `build` (release) |
| `sandbox-image` | `hurad/images` | `test`: the chat host |
| `hura-cloud` | `services/cloud` | `fmt`, `clippy`, `test`, `image` |
| `relay` | `services/relay` | `typecheck`, `test`, `dev` |
| `desktop` | `apps/desktop` | `types`, `bindings`, `tauri-versions`, `check-windows`, `dev`, `build` |
| `hurakit` | `apps/ios/HuraKit` | `test` |
| `ios` | `apps/ios` | `test`, `testflight` |

## How it is laid out

- `.moon/workspace.yml` names the projects. Each Rust project is named for its
  cargo package, because the shared tasks pass it to `cargo -p`.
- `.moon/tasks/rust.yml` is the `fmt`, `clippy` and `test` every crate in the
  cargo workspace inherits, run from the workspace root so they share one
  `target/`.
- Each project's `moon.yml` adds its own tasks, and the inputs that decide when
  they are affected.

Three things worth knowing before changing them:

- **A task is affected by its inputs, and only its inputs.** moon follows
  `dependsOn` between projects only through task `deps`, so a crate's tasks list
  the shared crates' sources as inputs themselves (the `shared` file group in
  `rust.yml`). That is also what keeps the cache honest: `hurad:test` must not be
  replayed after `hura-core` changed. A new file a task reads, like the push
  vector HuraKit tests against, belongs in its inputs.
- **`options.os` makes a task a no-op elsewhere.** Only the Windows checks and
  the iPhone tasks name an OS; everything else runs anywhere, so a Mac can run
  the Linux job's checks.
- **Releases are not tasks.** They are cut by tag (CONTRIBUTING.md), not by
  what changed.

## In CI

`.github/workflows/ci.yml` sets `MOON_BASE` to what the change is measured
against -- a pull request's base, or what `main` was before a push -- and runs
`moon ci` on Linux. The Windows and macOS jobs run only when `moon query tasks
--affected` says one of their tasks is affected, and run only those
(`:check-windows`, and `hurakit:test ios:test`). A manual dispatch measures
against git's empty tree, which makes every file changed and runs everything.
