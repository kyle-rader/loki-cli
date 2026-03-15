/// Environment variable for the branch name prefix used by `new` and `worktree add`.
pub const LOKI_NEW_PREFIX: &str = "LOKI_NEW_PREFIX";

/// Environment variable for the base ref used by `worktree add`.
pub const LOKI_WORKTREE_BASE: &str = "LOKI_WORKTREE_BASE";

/// Environment variable for the rebase target branch.
pub const LOKI_REBASE_TARGET: &str = "LOKI_REBASE_TARGET";

/// Git config override that disables all hooks.
pub const NO_HOOKS: &str = "core.hooksPath=/dev/null";
