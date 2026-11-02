nix := "nix develop --command"

# Default: list recipes
default:
    @just --list

# Required checks for structural changes (uses the same system libraries as build).
check:
    {{nix}} cargo fmt --all -- --check
    {{nix}} cargo test --workspace
    {{nix}} cargo clippy --workspace --all-targets
    git diff --check

# Build release inside the flake dev shell with runtime library paths.
build:
    #!/usr/bin/env bash
    {{nix}} bash <<'BUILD'
        set -euo pipefail

        # Embed Nix library paths for runtime loading outside the dev shell.
        # Missing Vulkan forces iced's CPU fallback, which strips alpha.
        rpath() {
            pkg-config --variable=libdir "$1"
        }

        rpaths="-Wl,-rpath,$(rpath wayland-client)"
        rpaths+=",-rpath,$(rpath xkbcommon)"
        rpaths+=",-rpath,$(rpath vulkan)"

        export RUSTFLAGS="-C link-args=$rpaths"
        cargo build --release
    BUILD

test: build
    if pgrep -x -u "$(id -u)" riced >/dev/null; then pkill -x -u "$(id -u)" riced; fi
    ./target/release/riced -p test

# Build release and install to ~/.local/bin (already on PATH)
install: build
    riced

# Remove the installed binary
uninstall:
    rm -f "{{ home_directory() }}/.local/bin/riced"

# Build the example system extension and install it into the test profile
extension-system:
    {{nix}} cargo build --release -p riced-extension-system
    install -Dm755 target/release/riced-system "{{ home_directory() }}/.config/riced/test/extensions/system/riced-system"

# Build the generic Wayland extension and install it into the test profile
extension-wayland:
    {{nix}} cargo build --release -p riced-extension-wayland
    install -Dm755 target/release/riced-wayland "{{ home_directory() }}/.config/riced/test/extensions/wayland/riced-wayland"

# Build the D-Bus tray/notification extension and install it into the test profile
extension-dbus:
    {{nix}} cargo build --release -p riced-extension-dbus
    install -Dm755 target/release/riced-dbus "{{ home_directory() }}/.config/riced/test/extensions/dbus/riced-dbus"
