/// Environment variable for the branch name prefix used by `new` and `worktree add`.
pub const LOKI_NEW_PREFIX: &str = "LOKI_NEW_PREFIX";

/// Environment variable for the base ref used by `worktree add`.
pub const LOKI_WORKTREE_BASE: &str = "LOKI_WORKTREE_BASE";

/// Git config override that disables all hooks.
pub const NO_HOOKS: &str = "core.hooksPath=/dev/null";
