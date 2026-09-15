# lx development tasks
# `make dev-setup` installs every tool the pre-commit hooks need.

.PHONY: dev-setup build test test-containers clippy fmt check doc policy func-tests bench

# Install all dev tools required by pre-commit hooks.
# Idempotent — skips anything already on PATH. Implemented by the Rust
# xtask (src/bin/xtask/dev.rs); no shell scripts live in this repo.
dev-setup:
	cargo xtask dev-setup

# Enforce the Rust-only policy (no shell scripts anywhere in the tree)
policy:
	cargo xtask policy

# Run the end-to-end CLI functional tests (needs a release binary)
func-tests:
	cargo build --release
	cargo xtask func-tests

# Benchmark lx against the tools it replaces
bench:
	cargo build --release
	cargo xtask bench

# Build the project
build:
	cargo build --all-targets --all-features

# Run all tests
test:
	cargo test --all-targets

# Run the opt-in container matrix (needs podman/docker; see tests/container_matrix.rs)
test-containers:
	cargo test --features container-tests --test container_matrix -- --nocapture

# Run clippy (deny warnings)
clippy:
	cargo clippy --all-targets --all-features -- -D warnings

# Check formatting
fmt:
	cargo fmt --all -- --check

# Run all pre-commit hooks on all files
check:
	pre-commit run --all-files

# Build docs (deny warnings)
doc:
	RUSTDOCFLAGS="--deny warnings" cargo doc --no-deps --all-features
