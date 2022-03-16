# `loki-cli` A Git Productivity CLI

Git is a pretty great tool on it's own. After some time common patterns emerge. `lk` is here to make those patterns fast.

# Install

1. First, install `cargo` by visiting https://rustup.rs.
2. Install with `cargo` ([📦 loki-cli ](https://crates.io/crates/loki-cli)):

    ```shell
    cargo install loki-cli
    ```

# Use
## Get Help
```shell
❯ lk --help
loki-cli 0.3.0
Kyle W. Rader
A CLI for Git Productivity

USAGE:
    lk.exe <SUBCOMMAND>

OPTIONS:
    -h, --help       Print help information
    -V, --version    Print version information

SUBCOMMANDS:
    fetch    Fetch with --prune deleting local branches pruned from the remote
    help     Print this message or the help of the given subcommand(s)
    new      Create a new branch from HEAD and push it to origin [aliases: n]
    pull     Pull with --prune deleting local branches pruned from the remote
    push     Push the current branch to origin with --set-upstream
```

## Commands

### `new`
Alias: `n`
* Make creating a new branch easier to type by joining all given args with a dash (`-`).
* Automatically push and setup tracking to `origin`.

#### Example
```
❯ lk new readme updates
```
Creates and pushes `readme-updates` to origin with `--set-upstream`. (The command git will tell you to run if you simply run `git push` after creating a new local branch.)

### `push`
Alias: none
* Pushes the current branch to origin with `--set-upstream`.
* `-f|--force` flag uses `--force-with-lease` under the hood for better force push safety.
* Only works if `HEAD` is on a branch (not in a dettached state).

### `pull`
Alias: none
* Run `git pull --prune` and remove any local branches that have also been pruned on the remote.

### `fetch`
Alias: none
* Run `git fetch --prune` and remove any local branches that have also been pruned on the remote.