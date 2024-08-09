# `loki-cli` A Git Productivity CLI

[![Cargo Test](https://github.com/kyle-rader/loki-cli/actions/workflows/pr-build.yml/badge.svg)](https://github.com/kyle-rader/loki-cli/actions/workflows/pr-build.yml)

Git is a pretty great tool on it's own. After some time common patterns emerge. `lk` is here to make those patterns fast.

# Install

1. First, install `cargo` by visiting https://rustup.rs.
2. Install with `cargo` ([📦 loki-cli ](https://crates.io/crates/loki-cli)):

    ```shell
    cargo install loki-cli
    ```

# Use
## Get Help
```
lk -h
Loki: 🚀 A Git productivity tool

Usage: lk <COMMAND>

Commands:
  new       Create a new branch from HEAD and push it to origin. Set a prefix for all new branch names with the env var LOKI_NEW_PREFIX [aliases: n]
  push      Push the current branch to origin with --set-upstream [aliases: p]
  pull      Pull with --prune deleting local branches pruned from the remote
  fetch     Fetch with --prune deleting local branches pruned from the remote
  save      Add, commit, and push using a timestamp based commit message [aliases: s]
  commit    Commit local changes [aliases: c]
  rebase    Rebase the current branch onto the target branch after fetching
  no-hooks  Run any command without triggering any hooks [aliases: x]
  help      Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

## Commands

### `new`
Alias: `n`
* Make creating a new branch easier to type by joining all given args with a dash (`-`).
* Automatically push and setup tracking to `origin`.
* Set a prefix to always prepend with the `LOKI_NEW_PREFIX` environment variable.

#### Example
```
❯ lk new readme updates
```
Creates and pushes `readme-updates` to origin with `--set-upstream`. (The command git will tell you to run if you simply run `git push` after creating a new local branch.)

### `save`
Alias: `s`

This is a wrapper around `lk commit + lk push`
* Commits current changes in tracked files (optionally all files with `--all`)
* Pushes via `lk push`

### `commit`
Alias: `c`
* Commits current changes in tracked files (optionally all files with `--all`)

### `push`
Alias: `p`
* Pushes the current branch to origin with `--set-upstream`.
* `-f|--force` flag uses `--force-with-lease` under the hood for better force push safety.
* Only works if `HEAD` is on a branch (not in a dettached state).

### `pull`
Alias: none (the alias `p` is for `push`)
* Run `git pull --prune` and remove any local branches that have also been pruned on the remote.

### `fetch`
Alias: none
* Run `git fetch --prune` and remove any local branches that have also been pruned on the remote.

### `rebase`
Fetch and rebase the current branch onto the target branch, or `main` by default.

### `no-hooks`
Alias: `x`

Execute a git commit without running any hooks

#### Example

```sh
lk x commit -m "Commit without any hooks"
```
