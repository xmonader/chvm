MUSL_TARGET := x86_64-unknown-linux-musl

.PHONY: build build-debug release test unit-test integration-test check fmt fmt-check clippy clean install doctor prune \
       docker-build docker-build-alpine docker-build-ubuntu docker-build-nginx docker-build-redis docker-build-ssh

# Default target
all: build

# Build all binaries statically linked with musl
build:
	cargo build --workspace --release --target $(MUSL_TARGET)

# Build debug binaries (also statically linked)
build-debug:
	cargo build --workspace --target $(MUSL_TARGET)

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

# Install binaries and shell completions
install: build
	mkdir -p ~/.cargo/bin
	cp target/$(MUSL_TARGET)/release/chvm ~/.cargo/bin/chvm
	doas mkdir -p ~/.chvm/bin
	doas cp target/$(MUSL_TARGET)/release/chvm-init ~/.chvm/bin/chvm-init
	mkdir -p ~/.local/share/bash-completion/completions
	COMPLETE=bash ~/.cargo/bin/chvm > ~/.local/share/bash-completion/completions/chvm
	cp scripts/doas-completion.bash ~/.local/share/bash-completion/completions/doas

# Run chvm doctor
doctor: build
	./target/$(MUSL_TARGET)/release/chvm doctor

# Prune stopped VMs and clean stale resources
prune: build
	doas ./target/$(MUSL_TARGET)/release/chvm prune

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
