#!/bin/sh
# Build and install UOC Journal from source for the current user.
#
#   ./install.sh                      from a checkout of this repository
#   curl -fsSL <raw url>/install.sh | sh   anywhere (clones to ~/.local/src)
#
# Installs the system libraries the app needs (asks for sudo), installs Rust
# with rustup if cargo is missing, builds a release binary, and puts it in
# ~/.local/bin with a menu entry. Run it again to update.
set -eu

REPO_URL="${UOC_JOURNAL_REPO:-https://github.com/13bfc3s/UOC-Journal.git}"
BIN="${XDG_BIN_HOME:-$HOME/.local/bin}"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"

say() { printf '\033[1m==> %s\033[0m\n' "$*"; }

# --- system packages -------------------------------------------------------
# Build: git, curl, a C toolchain, pkg-config.
# Runtime (loaded when the app starts): OpenGL/EGL, xkbcommon, Wayland, X11.
SUDO=""
[ "$(id -u)" -eq 0 ] || SUDO="sudo"
if command -v apt-get >/dev/null 2>&1; then
    say "Installing packages (apt)"
    $SUDO apt-get update
    $SUDO apt-get install -y git curl build-essential pkg-config \
        libegl1 libgl1 libxkbcommon0 libxkbcommon-x11-0 libwayland-client0 \
        libwayland-egl1 libx11-6 libxcursor1 libxrandr2 libxi6
elif command -v dnf >/dev/null 2>&1; then
    say "Installing packages (dnf)"
    $SUDO dnf install -y git curl gcc pkgconf-pkg-config \
        mesa-libEGL mesa-libGL libxkbcommon libxkbcommon-x11 libwayland-client \
        libwayland-egl libX11 libXcursor libXrandr libXi
elif command -v pacman >/dev/null 2>&1; then
    say "Installing packages (pacman)"
    $SUDO pacman -S --needed --noconfirm git curl base-devel pkgconf \
        libglvnd mesa libxkbcommon libxkbcommon-x11 wayland \
        libx11 libxcursor libxrandr libxi
elif command -v zypper >/dev/null 2>&1; then
    say "Installing packages (zypper)"
    $SUDO zypper --non-interactive install git curl gcc pkg-config \
        Mesa-libEGL1 Mesa-libGL1 libxkbcommon0 libxkbcommon-x11-0 \
        libwayland-client0 libwayland-egl1 libX11-6 libXcursor1 libXrandr2 libXi6
else
    say "Unknown package manager: install git, curl, a C compiler, OpenGL/EGL,"
    say "libxkbcommon(-x11), libwayland-client and the X11 libraries yourself."
fi

# --- Rust ------------------------------------------------------------------
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
if ! command -v cargo >/dev/null 2>&1; then
    say "Installing Rust (rustup)"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
    . "$HOME/.cargo/env"
fi

# --- source ----------------------------------------------------------------
HERE="$(cd "$(dirname "$0")" 2>/dev/null && pwd || true)"
if [ -n "${UOC_JOURNAL_SRC:-}" ]; then
    SRC="$UOC_JOURNAL_SRC"
elif [ -n "$HERE" ] && [ -f "$HERE/crates/app/Cargo.toml" ]; then
    SRC="$HERE" # running from a checkout: build it as it is
else
    SRC="$HOME/.local/src/uoc-journal"
    if [ -d "$SRC/.git" ]; then
        say "Updating $SRC"
        git -C "$SRC" pull --ff-only
    else
        say "Cloning into $SRC"
        mkdir -p "$(dirname "$SRC")"
        git clone --depth 1 "$REPO_URL" "$SRC"
    fi
fi

# --- build & install -------------------------------------------------------
say "Building (first build takes a few minutes)"
cargo build --release --locked -p uoc-journal --manifest-path "$SRC/Cargo.toml"

say "Installing"
mkdir -p "$BIN" "$DATA/applications" "$DATA/icons/hicolor/64x64/apps"
install -m 755 "$SRC/target/release/uoc-journal" "$BIN/uoc-journal"
install -m 644 "$SRC/packaging/uoc-journal.png" "$DATA/icons/hicolor/64x64/apps/uoc-journal.png"
sed "s|^Exec=.*|Exec=$BIN/uoc-journal|" "$SRC/packaging/uoc-journal.desktop" \
    > "$DATA/applications/uoc-journal.desktop"
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$DATA/applications" || true

say "Done: $BIN/uoc-journal (menu entry: UOC Journal)"
case ":$PATH:" in
*":$BIN:"*) ;;
*) echo "Note: $BIN is not on your PATH; run it as $BIN/uoc-journal or add it to PATH." ;;
esac
