# Contributing to BrainWashed

Thanks for helping! A few conventions keep the monorepo healthy.

## Before you open a pull request

```sh
pnpm typecheck
pnpm test
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI runs the same checks and builds the desktop host on macOS, Windows and Linux.

## Conventions

- Shared wire types live in `packages/api`. When you change one, update the matching Rust struct (they are marked with a comment pointing at each other).
- Keep pull requests focused on one change.
- By contributing you agree your work is licensed under Apache-2.0.
