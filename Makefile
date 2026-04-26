.PHONY: all check test clippy fmt fmt-check build release harness install uninstall clean

PREFIX ?= $(HOME)/.cargo
BIN    := lg

all: check test clippy fmt-check

check:
	cargo check --all-targets

test:
	cargo test --all-targets

clippy:
	cargo clippy --all-targets -- -D warnings

fmt:
	cargo fmt

fmt-check:
	cargo fmt --check

build:
	cargo build

release:
	cargo build --release

harness:
	cargo run --bin harness

install: release
	install -d $(PREFIX)/bin
	install -m 0755 target/release/$(BIN) $(PREFIX)/bin/$(BIN)

uninstall:
	rm -f $(PREFIX)/bin/$(BIN)

clean:
	cargo clean
