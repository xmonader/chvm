MUSL_TARGET := x86_64-unknown-linux-musl

.PHONY: build build-debug release test unit-test integration-test check fmt fmt-check clippy clean install doctor prune init

# Default target
all: build

# Build chvm-init with musl (statically linked for guest VMs)
init:
	cargo build -p chvm-init --release --target $(MUSL_TARGET)

# Build release binaries (host + guest init)
build: init
	cargo build --workspace --release

# Build debug binaries
build-debug:
	cargo build -p chvm-init --target $(MUSL_TARGET)
	cargo build --workspace

# Alias for build
release: build

# Run unit tests
unit-test:
	cargo test --workspace

# Run integration tests (requires root)
integration-test:
	doas bash tests/run_tests.sh

# Run all tests: unit + integration
test: unit-test integration-test

# Run cargo check (fast compile check)
check:
	cargo check --workspace

# Format code
fmt:
	cargo fmt --all

# Format check (CI)
fmt-check:
	cargo fmt --all -- --check

# Run clippy lints
clippy:
	cargo clippy --workspace -- -D warnings

# Install binary to ~/.cargo/bin
install:
	cargo install --path crates/chvm-cli

# Run chvm doctor
doctor: build
	./target/release/chvm doctor

# Prune stopped VMs and clean stale resources
prune: build
	doas ./target/release/chvm prune

# Clean build artifacts
clean:
	cargo clean
