MUSL_TARGET := x86_64-unknown-linux-musl

.PHONY: build build-debug release test unit-test integration-test check fmt fmt-check clippy clean install doctor prune init \
       docker-build docker-build-alpine docker-build-ubuntu docker-build-nginx docker-build-redis docker-build-ssh

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

# Install binaries to ~/.cargo/bin and ~/.chvm/bin
install: init
	cargo install --path crates/chvm-cli
	mkdir -p ~/.chvm/bin
	cp target/$(MUSL_TARGET)/release/chvm-init ~/.chvm/bin/chvm-init

# Run chvm doctor
doctor: build
	./target/release/chvm doctor

# Prune stopped VMs and clean stale resources
prune: build
	doas ./target/release/chvm prune

# Clean build artifacts
clean:
	cargo clean

# --- Docker example images ---

DOCKER_DIR := dockers

# Build all example Docker images
docker-build: docker-build-alpine docker-build-ubuntu docker-build-nginx docker-build-redis docker-build-ssh

# Build individual images
docker-build-alpine:
	docker build -t localhost/chvm-alpine-hello:latest $(DOCKER_DIR)/alpine-hello

docker-build-ubuntu:
	docker build -t localhost/chvm-ubuntu-dev:latest $(DOCKER_DIR)/ubuntu-dev

docker-build-nginx:
	docker build -t localhost/chvm-nginx-web:latest $(DOCKER_DIR)/nginx-web

docker-build-redis:
	docker build -t localhost/chvm-redis-cache:latest $(DOCKER_DIR)/redis-cache

docker-build-ssh:
	docker build -t localhost/chvm-alpine-ssh:latest $(DOCKER_DIR)/alpine-ssh
