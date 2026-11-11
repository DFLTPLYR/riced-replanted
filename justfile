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
    install -Dm755 "target/release/riced" "{{ home_directory() }}/.local/bin/riced"
    riced ipc restart

test: build
    if pgrep -x -u "$(id -u)" riced >/dev/null; then pkill -x -u "$(id -u)" riced; fi
    ./target/release/riced --logs

# Build release and install to ~/.local/bin (already on PATH)
install: build
    install -Dm755 target/release/riced "{{ home_directory() }}/.local/bin/riced"

# Build, install, then re-exec the running daemon in place (same PID)
restart: install
    riced ipc restart

# Remove the installed binary
uninstall:
    rm -f "{{ home_directory() }}/.local/bin/riced"

# Install the LuaLS definitions into a profile (default: replanted)
stubs profile="replanted":
    riced stubs

# Build and install every extension into the default profile
build-extensions:
    #!/usr/bin/env bash
    set -euo pipefail
    output="${XDG_CONFIG_HOME:-$HOME/.config}/riced/extensions"
    for extension in applications dbus lucide openmeteo system themeagen wayland; do
        {{nix}} riced extension build "extensions/$extension" -o "$output/$extension"
    done

# Build the example system extension and install it into the default profile
extension-system:
    {{nix}} cargo build --release -p riced-extension-system
    install -Dm755 target/release/riced-system "{{ home_directory() }}/.config/riced/extensions/system/riced-system"
    install -Dm644 extensions/system/stub.lua "{{ home_directory() }}/.config/riced/extensions/system/stub.lua"

# Build the generic Wayland extension and install it into the default profile
extension-wayland:
    {{nix}} cargo build --release -p riced-extension-wayland
    install -Dm755 target/release/riced-wayland "{{ home_directory() }}/.config/riced/extensions/wayland/riced-wayland"
    install -Dm644 extensions/wayland/stub.lua "{{ home_directory() }}/.config/riced/extensions/wayland/stub.lua"

# Build the D-Bus tray/notification extension and install it into the default profile
extension-dbus:
    {{nix}} cargo build --release -p riced-extension-dbus
    install -Dm755 target/release/riced-dbus "{{ home_directory() }}/.config/riced/extensions/dbus/riced-dbus"
    install -Dm644 extensions/dbus/stub.lua "{{ home_directory() }}/.config/riced/extensions/dbus/stub.lua"

# Build the Material You theme generator and install it into the default profile
extension-themeagen:
    {{nix}} cargo build --release -p riced-extension-themeagen
    install -Dm755 target/release/riced-themeagen "{{ home_directory() }}/.config/riced/extensions/themeagen/riced-themeagen"
    install -Dm644 extensions/themeagen/stub.lua "{{ home_directory() }}/.config/riced/extensions/themeagen/stub.lua"

# Build the Lucide icon extension and install it into the default profile
extension-lucide:
    {{nix}} cargo build --release -p riced-extension-lucide
    install -Dm755 target/release/riced-lucide "{{ home_directory() }}/.config/riced/extensions/lucide/riced-lucide"
    install -Dm644 extensions/lucide/lucide.lua "{{ home_directory() }}/.config/riced/extensions/lucide/lucide.lua"
    install -Dm644 extensions/lucide/stub.lua "{{ home_directory() }}/.config/riced/extensions/lucide/stub.lua"

# Build the application entries extension and install it into the default profile
extension-applications:
    {{nix}} cargo build --release -p riced-extension-applications
    install -Dm755 target/release/riced-applications "{{ home_directory() }}/.config/riced/extensions/applications/riced-applications"
    install -Dm644 extensions/applications/stub.lua "{{ home_directory() }}/.config/riced/extensions/applications/stub.lua"

# Build the Open-Meteo weather extension and its Lua widget
extension-openmeteo:
    {{nix}} cargo build --release -p riced-extension-openmeteo
    install -Dm755 target/release/riced-openmeteo "{{ home_directory() }}/.config/riced/extensions/openmeteo/riced-openmeteo"
    install -Dm644 extensions/openmeteo/openmeteo.lua "{{ home_directory() }}/.config/riced/extensions/openmeteo/openmeteo.lua"
    install -Dm644 extensions/openmeteo/stub.lua "{{ home_directory() }}/.config/riced/extensions/openmeteo/stub.lua"
