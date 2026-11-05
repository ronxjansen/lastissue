.PHONY: build install uninstall dev bundle bench test

DESKTOP := apps/desktop

build:
	cargo build --release -p cli
	pnpm -C $(DESKTOP) install --frozen-lockfile
	pnpm -C $(DESKTOP) tauri build --no-bundle

# Installs `lastissue` into ~/.cargo/bin (must be on PATH). The desktop app
# installs as a bundle: `make bundle`, then copy the .app to /Applications.
install:
	cargo install --locked --force --path crates/cli

uninstall:
	cargo uninstall cli

# The desktop app with hot reload (Vite + cargo).
dev:
	pnpm -C $(DESKTOP) install
	pnpm -C $(DESKTOP) tauri dev

# Builds target/release/bundle/macos/lastissue.app and a .dmg (unsigned).
bundle:
	pnpm -C $(DESKTOP) install --frozen-lockfile
	pnpm -C $(DESKTOP) tauri build

test:
	cargo test --workspace
	pnpm -C $(DESKTOP) check
	pnpm -C $(DESKTOP) test

# Scale benchmarks (10k/100k synthetic issues). Stores go to
# LASTISSUE_BENCH_DIR, else target/tmp/lastissue-bench.
bench:
	cargo bench -p lastissue-core --bench scale
