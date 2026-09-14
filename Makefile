# lx development tasks
# `make dev-setup` installs every tool the pre-commit hooks need.

.PHONY: dev-setup build test clippy fmt check doc

# Install all dev tools required by pre-commit hooks.
# Idempotent — skips anything already on PATH.
dev-setup:
	@echo "==> Installing dev tools for lx..."

	@command -v pre-commit >/dev/null 2>&1 && \
		echo "  pre-commit: already installed" || \
		(echo "  pre-commit: installing..." && pip install pre-commit)

	@command -v cargo-deny >/dev/null 2>&1 && \
		echo "  cargo-deny: already installed" || \
		(echo "  cargo-deny: installing..." && cargo install cargo-deny)

	@command -v typos >/dev/null 2>&1 && \
		echo "  typos: already installed" || \
		(echo "  typos: installing..." && cargo install typos-cli)

	@command -v taplo >/dev/null 2>&1 && \
		echo "  taplo: already installed" || \
		(echo "  taplo: installing..." && cargo install taplo-cli --locked)

	@command -v markdownlint >/dev/null 2>&1 && \
		echo "  markdownlint: already installed" || \
		(echo "  markdownlint: installing..." && npm install -g markdownlint-cli)

	@command -v shellcheck >/dev/null 2>&1 && \
		echo "  shellcheck: already installed" || \
		(echo "  shellcheck: MISSING — install via your package manager (apt/brew)" >&2)

	@echo ""
	@echo "==> Done. Now run:"
	@echo "    pre-commit install"
	@echo "    pre-commit install --hook-type pre-push"

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
