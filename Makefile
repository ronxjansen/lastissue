.PHONY: build install uninstall dev bundle bench test

DESKTOP := crates/desktop

APP := target/release/bundle/macos/lastissue.app

# The CLI, and the desktop app as a .app bundle (no .dmg; see `bundle`).
build:
	cargo build --release -p cli
	pnpm -C $(DESKTOP) install --frozen-lockfile
	pnpm -C $(DESKTOP) tauri build --bundles app

# Installs `lastissue` into ~/.cargo/bin (must be on PATH) and the app
# built by `make build` into /Applications, replacing the old one. Quit
# and reopen the app to pick it up.
install:
	cargo install --locked --force --path crates/cli
	@test -d $(APP) || { echo "no $(APP): run make build first"; exit 1; }
	rm -rf /Applications/lastissue.app
	cp -R $(APP) /Applications/

uninstall:
	cargo uninstall cli
	rm -rf /Applications/lastissue.app

# The desktop app with hot reload (Vite + cargo).
dev:
	pnpm -C $(DESKTOP) install
	pnpm -C $(DESKTOP) tauri dev

# Builds $(APP) and a .dmg (unsigned), for sharing.
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
