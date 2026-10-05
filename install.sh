#!/bin/sh
# Install (or update) UOC Journal from source. Paste this into a terminal:
#
#   sh -c "$(curl -fsSL https://raw.githubusercontent.com/13bfc3s/UOC-Journal/main/install.sh || wget -qO- https://raw.githubusercontent.com/13bfc3s/UOC-Journal/main/install.sh)"
#
# It installs git, a C compiler and the libraries the app needs (asks for
# sudo), installs Rust with rustup if cargo is missing, clones the repository
# to ~/.local/src/uoc-journal (or updates it), builds it, installs it to
# ~/.local/bin with a menu entry, and starts it. Run it again to update.
#
# Run from inside a checkout (./install.sh), it builds that checkout instead.
# Set UOC_JOURNAL_NO_LAUNCH=1 to skip starting the app at the end.
set -eu

REPO_URL="${UOC_JOURNAL_REPO:-https://github.com/13bfc3s/UOC-Journal.git}"
BIN="${XDG_BIN_HOME:-$HOME/.local/bin}"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"

say() { printf '\033[1m==> %s\033[0m\n' "$*"; }
die() {
    printf '\033[1;31mError: %s\033[0m\n' "$*" >&2
    exit 1
}

# Build: git, curl, a C toolchain, pkg-config.
# Runtime (loaded when the app starts): OpenGL/EGL, xkbcommon, Wayland, X11.
install_packages() {
    SUDO=""
    if [ "$(id -u)" -ne 0 ]; then
        command -v sudo >/dev/null 2>&1 || die "sudo is needed to install packages"
        SUDO="sudo"
    fi
    if command -v apt-get >/dev/null 2>&1; then
        say "Installing packages (apt)"
        $SUDO apt-get update
        $SUDO apt-get install -y git curl ca-certificates build-essential pkg-config \
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
        say "Unknown package manager. Make sure git, curl, a C compiler, OpenGL/EGL,"
        say "libxkbcommon(-x11), libwayland-client and the X11 libraries are installed."
    fi
    for tool in git curl cc; do
        command -v "$tool" >/dev/null 2>&1 || die "'$tool' is still missing; install it and run this again"
    done
}

# The dependencies need Rust 1.95 or newer; distro packages are often older.
RUST_MINOR=95
rust_ok() {
    v="$($CARGO --version 2>/dev/null | sed -n 's/^cargo 1\.\([0-9]*\).*/\1/p')"
    [ -n "$v" ] && [ "$v" -ge "$RUST_MINOR" ]
}

# Sets CARGO to a cargo command that is new enough.
install_rust() {
    if [ -f "$HOME/.cargo/env" ]; then . "$HOME/.cargo/env"; fi
    CARGO="cargo"
    rust_ok && return 0
    if command -v rustup >/dev/null 2>&1; then
        say "Updating Rust (rustup)"
        rustup toolchain install stable --profile minimal
        CARGO="cargo +stable"
    else
        say "Installing Rust (rustup)"
        # rustup-init picks its mode from its file name, so keep that name.
        tmp="$(mktemp -d)"
        url="https://static.rust-lang.org/rustup/dist/$(uname -m)-unknown-linux-gnu/rustup-init"
        curl --proto '=https' --tlsv1.2 -fsSL -o "$tmp/rustup-init" "$url" || die "could not download $url"
        chmod +x "$tmp/rustup-init"
        "$tmp/rustup-init" -y --profile minimal
        rm -rf "$tmp"
        . "$HOME/.cargo/env"
        CARGO="cargo +stable"
    fi
    rust_ok || die "Rust 1.$RUST_MINOR or newer is needed (found: $($CARGO --version 2>&1))"
}

# Sets SRC to the checkout to build.
get_source() {
    here="$(cd "$(dirname "$0")" 2>/dev/null && pwd)" || here=""
    if [ -n "${UOC_JOURNAL_SRC:-}" ]; then
        SRC="$UOC_JOURNAL_SRC"
    elif [ -n "$here" ] && [ -f "$here/crates/app/Cargo.toml" ] && [ -f "$here/install.sh" ]; then
        SRC="$here"
        say "Using the checkout in $SRC"
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
}

build_and_install() {
    say "Building (the first build takes a few minutes)"
    $CARGO build --release --locked -p uoc-journal --manifest-path "$SRC/Cargo.toml"

    say "Installing"
    mkdir -p "$BIN" "$DATA/applications" "$DATA/icons/hicolor/64x64/apps"
    install -m 755 "$SRC/target/release/uoc-journal" "$BIN/uoc-journal"
    install -m 644 "$SRC/packaging/uoc-journal.png" "$DATA/icons/hicolor/64x64/apps/uoc-journal.png"
    sed "s|^Exec=.*|Exec=$BIN/uoc-journal|" "$SRC/packaging/uoc-journal.desktop" \
        >"$DATA/applications/uoc-journal.desktop"
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$DATA/applications" || true
    fi
    say "Installed $BIN/uoc-journal (menu entry: UOC Journal)"
    case ":$PATH:" in
    *":$BIN:"*) ;;
    *) echo "Note: $BIN is not on your PATH; run it as $BIN/uoc-journal or add it to PATH." ;;
    esac
}

launch() {
    [ -z "${UOC_JOURNAL_NO_LAUNCH:-}" ] || return 0
    if [ -n "${WAYLAND_DISPLAY:-}" ] || [ -n "${DISPLAY:-}" ]; then
        say "Starting UOC Journal"
        nohup "$BIN/uoc-journal" >/dev/null 2>&1 &
    fi
}

# Everything runs from main, so the whole script is read before anything
# runs (safe with `curl ... | sh`).
main() {
    install_packages </dev/null
    install_rust </dev/null
    get_source
    build_and_install
    launch
}

main "$@"
