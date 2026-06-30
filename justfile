bin_ext := if os() == "windows" { ".exe" } else { "" }

build-dev:
    cargo build

build-release:
    cargo build --release

install: build-release
    target/release/claude_status_line{{bin_ext}} --install

check:
    cargo clippy -- -D warnings
    cargo test
