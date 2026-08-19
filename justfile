prefix := env_var_or_default("PREFIX", env_var("HOME") / ".cargo")
bin := "lg"

# check, test, clippy, and formatting checks
default: check test clippy fmt-check

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
    install -d {{prefix}}/bin
    install -m 0755 target/release/{{bin}} {{prefix}}/bin/{{bin}}

uninstall:
    rm -f {{prefix}}/bin/{{bin}}

clean:
    cargo clean
