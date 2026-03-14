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
  new        Create a new branch from HEAD and push it to origin. Set a prefix with --prefix or the LOKI_NEW_PREFIX env var [aliases: n]
  push       Push the current branch to origin with --set-upstream [aliases: p]
  pull       Pull with --prune deleting local branches pruned from the remote
  fetch      Fetch with --prune deleting local branches pruned from the remote
  save       Add, commit, and push using a timestamp based commit message [aliases: s]
  commit     Commit local changes [aliases: c]
  rebase     Rebase the current branch onto the target branch after fetching
  worktree   Manage git worktrees [aliases: w]
  no-hooks   Run any command without triggering any hooks [aliases: x]
  help       Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

## Commands

### `new`
Alias: `n`
* Make creating a new branch easier to type by joining all given args with a dash (`-`).
* Automatically push and setup tracking to `origin`.
* Set a prefix to always prepend with the `--prefix` flag or the `LOKI_NEW_PREFIX` environment variable.

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
lk x -- commit -m "Update Readme without running hooks"
```

### `worktree`
Alias: `w`

Manage git worktrees for parallel development workflows. Subcommands:

#### `worktree add <name>` (alias: `a`)
Create a new worktree as a sibling directory and set up a branch with upstream tracking.

```sh
# Creates worktree at ../my-project_fix-auth, creates and pushes branch
❯ lk w a fix-auth

# With a custom base ref
❯ lk w a fix-auth --base origin/develop

# With a branch prefix (via flag or LOKI_NEW_PREFIX env var)
❯ lk w a fix-auth --prefix users/danigon/
```

The worktree is created at `<parent>/<repo>_<name>` (e.g., `~/repos/my-project_fix-auth`).

**Flags:**
- `--base` / `-b` — Base ref (default: `origin/main`, env: `LOKI_WORKTREE_BASE`)
- `--prefix` — Branch name prefix (env: `LOKI_NEW_PREFIX`)

#### `worktree remove [name]` (alias: `r`)
Remove a worktree and delete its local branch.

```sh
# From inside the worktree — name is inferred from the directory
~/repos/my-project_fix-auth ❯ lk w r

# Explicit name from anywhere
~/repos/my-project ❯ lk w r fix-auth

# Force remove a dirty worktree
❯ lk w r fix-auth --force
```

**Flags:**
- `--force` / `-f` — Force removal of dirty worktrees
- `--prefix` — Branch name prefix used during creation (env: `LOKI_NEW_PREFIX`)

#### `worktree list` (alias: `l`)
List all worktrees. Highlights the current worktree in green.

```sh
❯ lk w l
```

#### `worktree switch [name]` (alias: `s`)
Print a `cd` command for switching to a worktree. Designed for use with `eval`:

```bash
# bash/zsh — switch to a named worktree
eval "$(lk w s fix-auth)"

# PowerShell
lk w s fix-auth | Invoke-Expression

# Switch to the main worktree (no name)
eval "$(lk w s)"
```

#### Shell Wrappers

All worktree commands output `cd <path>` to stdout (info goes to stderr), so you can
pipe to your shell for automatic directory switching:

```powershell
# PowerShell
lk w a fix-auth | Invoke-Expression
```

```bash
# bash/zsh
eval "$(lk w a fix-auth)"
```

### `repo stats`
Analyze commits reachable from HEAD to see who has been landing work in a repository. All of the filtering flags operate on commit dates.

- `--name` filters by author display name (repeatable, case-insensitive).
- `--email` filters by author email (repeatable, case-insensitive).
- `--all` includes all commits (default is first-parent only).

#### Example
```
❯ lk repo stats --weeks 4 --top 5
```
