#!/usr/bin/env bash
set -euo pipefail

REPO="https://github.com/CloudDown/omacourses/archive/refs/heads/main.tar.gz"

fail() {
  printf 'install.sh: %s\n' "$*" >&2
  exit 1
}

command -v tar >/dev/null 2>&1 || fail "tar is required."
if command -v curl >/dev/null 2>&1; then
  download() { curl -fsSL "$1"; }
elif command -v wget >/dev/null 2>&1; then
  download() { wget -qO- "$1"; }
else
  fail "Install curl or wget, then run this script again."
fi

if command -v pacman >/dev/null 2>&1 && [[ -r /etc/os-release ]] && grep -Eiq '^(ID|ID_LIKE)=.*(arch|omarchy)' /etc/os-release; then
  packages=(base-devel pkgconf rust wayland libxkbcommon libx11 libxcursor libxrandr libxi)
  if ! pacman -T "${packages[@]}" >/dev/null 2>&1; then
    printf 'Cahier needs build tools and graphics libraries. Install them with pacman? [y/N] '
    [[ -t 0 ]] || fail "Missing Arch dependencies; run interactively to install them."
    read -r answer
    [[ "$answer" =~ ^[Yy]$ ]] || fail "Required dependencies were not installed."
    if (( EUID == 0 )); then
      pacman -S --needed "${packages[@]}"
    elif command -v sudo >/dev/null 2>&1; then
      sudo pacman -S --needed "${packages[@]}"
    else
      fail "Install these packages as root: ${packages[*]}"
    fi
  fi
fi

command -v cargo >/dev/null 2>&1 || fail "Rust/Cargo is required. On Arch, install it with: sudo pacman -S rust"

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT
printf 'Downloading Cahier source…\n'
download "$REPO" | tar -xz -C "$workdir" --strip-components=1

printf 'Building Cahier (first build can take a few minutes)…\n'
(
  cd "$workdir"
  cargo build --locked --release --bin cahier
)

install_dir="${CARGO_INSTALL_ROOT:-$HOME/.local}/bin"
mkdir -p "$install_dir"
install -m 755 "$workdir/target/release/cahier" "$install_dir/cahier"
printf '\nInstalled Cahier to %s/cahier\n' "$install_dir"
if [[ ":$PATH:" != *":$install_dir:"* ]]; then
  printf 'Add it to your PATH if needed: export PATH="%s:$PATH"\n' "$install_dir"
fi
printf 'Run it with: cahier\n'
