## Agent Contribution Guide

Whenever an automated agent lands code in this repository, it must also:

- Bump the crate version in `Cargo.toml` before shipping.
- Choose the bump size using semantic versioning rules:
  - Increment the **major** version (`x.0.0`) for breaking changes.
  - Increment the **minor** version when adding backward-compatible functionality.
  - Increment the **patch** version for bug fixes and other backward-compatible maintenance work.

If you are unsure which category applies, err on the side of a **minor** bump and leave a note in your PR with the reasoning.
