#!/bin/sh
# Installs the BrainWashed command-line host on macOS or Linux and starts it:
#
#   curl -fsSL https://raw.githubusercontent.com/ahmadalshouly/brainwashed/main/install.sh | sh
#
# It downloads `brainwashed` from the newest GitHub release (pre-releases
# included) into ~/.local/bin. Set BRAINWASHED_VERSION=v0.1.0 to pick a
# release, or BRAINWASHED_NO_START=1 to only install.
set -eu

repo=ahmadalshouly/brainwashed
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) target=aarch64-apple-darwin ;;
  Darwin-x86_64) target=x86_64-apple-darwin ;;
  Linux-x86_64) target=x86_64-unknown-linux-gnu ;;
  Linux-aarch64 | Linux-arm64) target=aarch64-unknown-linux-gnu ;;
  *) echo "Sorry, there's no BrainWashed build for $(uname -s) $(uname -m) yet." >&2; exit 1 ;;
esac
asset="brainwashed-$target.tar.gz"

if [ -n "${BRAINWASHED_VERSION:-}" ]; then
  api="https://api.github.com/repos/$repo/releases/tags/$BRAINWASHED_VERSION"
else
  api="https://api.github.com/repos/$repo/releases?per_page=20"
fi
# Drafts have no public download links, so the first link found is the newest published one.
url=$(curl -fsSL "$api" | grep -o "\"browser_download_url\": *\"[^\"]*/$asset\"" | head -n 1 | sed 's/.*"\(https[^"]*\)"/\1/')
if [ -z "$url" ]; then
  echo "No BrainWashed release with a command-line build for your computer was found yet." >&2
  echo "See https://github.com/$repo/releases" >&2
  exit 1
fi

bin="$HOME/.local/bin"
mkdir -p "$bin"
echo "Downloading $url"
curl -fsSL "$url" | tar -xz -C "$bin" brainwashed
chmod +x "$bin/brainwashed"
echo "Installed $bin/brainwashed. Run 'brainwashed --help' to see what it can do."
case ":$PATH:" in
  *":$bin:"*) ;;
  *) echo "Add $bin to your PATH to run 'brainwashed' from any terminal." ;;
esac

if [ -z "${BRAINWASHED_NO_START:-}" ]; then
  # Piped into sh, stdin is the script; give the app the terminal instead.
  if [ -r /dev/tty ]; then exec "$bin/brainwashed" </dev/tty; else exec "$bin/brainwashed" --no-browser; fi
fi
