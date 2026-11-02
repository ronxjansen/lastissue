.PHONY: build install uninstall bundle bench

build:
	cargo build --release -p cli -p gui

# Installs `lastissue` and `lastissue-gui` into ~/.cargo/bin (must be on PATH).
install:
	cargo install --locked --force --path crates/cli
	cargo install --locked --force --path crates/gui

uninstall:
	cargo uninstall cli gui

# Builds target/release/bundle/lastissue.app (macOS, ad-hoc signed; not
# installed or registered). Copy it to /Applications to install.
bundle:
	crates/gui/macos/bundle.sh

# Scale benchmarks (10k/100k synthetic issues). Stores go to
# LASTISSUE_BENCH_DIR, else target/tmp/lastissue-bench.
bench:
	cargo bench -p lastissue-core --bench scale
