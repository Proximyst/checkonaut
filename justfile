# Fix all linting errors automatically, if possible.
fix:
    cargo clippy --fix --allow-dirty
    cargo fmt
    taplo fmt

# Replicate a full CI run.
ci:
    cargo fmt --check
    cargo clippy --all-targets --all-features -- -D warnings
    taplo fmt --check
    cargo build --all-targets --all-features
    cargo nextest run --all-features
    uvx zizmor .

# Set up a pre-commit hook to run `just ci`.
pre-commit:
    echo '#!/bin/sh' > .git/hooks/pre-commit
    echo 'just _pre-commit' >> .git/hooks/pre-commit
    chmod +x .git/hooks/pre-commit

_pre-commit: ci
