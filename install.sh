#!/bin/sh
# ReviewGate 安装脚本。
#   curl -fsSL https://raw.githubusercontent.com/dengmengmian/ReviewGate/main/install.sh | sh
# 可选环境变量：
#   REVIEWGATE_VERSION  指定版本（默认 latest）
#   REVIEWGATE_INSTALL_DIR  安装目录（默认 /usr/local/bin，无权限时回退 ~/.local/bin）
set -eu

REPO="dengmengmian/ReviewGate"
VERSION="${REVIEWGATE_VERSION:-latest}"
INSTALL_DIR="${REVIEWGATE_INSTALL_DIR:-/usr/local/bin}"

os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Linux)  os="linux" ;;
  Darwin) os="darwin" ;;
  *) echo "不支持的系统：$os" >&2; exit 1 ;;
esac
case "$arch" in
  x86_64|amd64) arch="x64" ;;
  arm64|aarch64) arch="arm64" ;;
  *) echo "不支持的架构：$arch" >&2; exit 1 ;;
esac
asset="reviewgate-$os-$arch"

if [ "$VERSION" = "latest" ]; then
  base_url="https://github.com/$REPO/releases/latest/download"
else
  base_url="https://github.com/$REPO/releases/download/$VERSION"
fi
url="$base_url/$asset"
sum_url="$base_url/sha256sum.txt"

tmp="$(mktemp)"
sum_tmp="$(mktemp)"
cleanup() {
  rm -f "$tmp" "$sum_tmp"
}
trap cleanup EXIT
echo "下载 $url …"
curl -fsSL "$url" -o "$tmp"
curl -fsSL "$sum_url" -o "$sum_tmp"
expected="$(awk -v asset="$asset" '$2 == asset { print $1 }' "$sum_tmp")"
if [ -z "$expected" ]; then
  echo "sha256sum.txt 中找不到 $asset" >&2
  exit 1
fi
actual="$(shasum -a 256 "$tmp" | awk '{ print $1 }')"
printf '%s  %s\n' "$expected" "$tmp" | shasum -a 256 -c -
if [ "$actual" != "$expected" ]; then
  echo "checksum mismatch for $asset" >&2
  exit 1
fi
chmod +x "$tmp"

if [ ! -w "$INSTALL_DIR" ] 2>/dev/null; then
  if mkdir -p "$INSTALL_DIR" 2>/dev/null && [ -w "$INSTALL_DIR" ]; then
    :
  else
    INSTALL_DIR="$HOME/.local/bin"
    mkdir -p "$INSTALL_DIR"
    echo "无写权限，改装到 ${INSTALL_DIR}（请确保它在 PATH 中）"
  fi
fi

mv "$tmp" "$INSTALL_DIR/reviewgate"
trap - EXIT
rm -f "$sum_tmp"
echo "已安装到 $INSTALL_DIR/reviewgate"
"$INSTALL_DIR/reviewgate" --version || true
