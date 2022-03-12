# `loki-cli` A Git Productivity CLI

Git is a pretty great tool on it's own. After some time common patterns emerge. `lk` is here to make those patterns fast.

# Install

1. First, install `cargo` by visiting https://rustup.rs.
2. Install with `cargo`:

    ```shell
    cargo install loki-cli
    ```

# Use
## Get Help
```shell
❯ lk --help
loki-cli 0.1.2
Kyle W. Rader
A CLI for Git Productivity

USAGE:
    lk.exe <SUBCOMMAND>

OPTIONS:
    -h, --help       Print help information
    -V, --version    Print version information

SUBCOMMANDS:
    help    Print this message or the help of the given subcommand(s)
    new     Create a new branch and push it to origin. All values given are joined with a "-" to
                form a valid git branch name. e.g. "lk new cool branch" creates "cool-branch"
                [aliases: n]
```

## Commands

### `new` (`n`)
* Make creating a new branch easier to type by joining all given args with a dash (`-`).
* Automatically push and setup tracking to `origin`.

#### Example
```
❯ lk new readme updates
Switched to a new branch 'readme-updates'
Total 0 (delta 0), reused 0 (delta 0), pack-reused 0
remote:
remote: Create a pull request for 'readme-updates' on GitHub by visiting:
remote:      https://github.com/kyle-rader/loki-cli/pull/new/readme-updates
remote:
To github.com:kyle-rader/loki-cli.git
 * [new branch]      readme-updates -> readme-updates
branch 'readme-updates' set up to track 'origin/readme-updates'.
```