#!/usr/bin/env bash
# PeerBanHelper-Rust 构建/测试脚本。
#
# 用法:
#   ./build.sh            构建发布版二进制 (target/release/pbh)
#   ./build.sh build      同上
#   ./build.sh debug      构建调试版 (更快, target/debug/pbh)
#   ./build.sh run        构建并运行 (调试版, 数据目录 ./data)
#   ./build.sh test       运行全部单元测试
#   ./build.sh clippy     运行 clippy + fmt 检查
#   ./build.sh package    构建发布版并打包到 dist/pbh-rust-<ver>-<os>-<arch>.tar.gz
#   ./build.sh clean      清理 target/ 与 dist/
#
# 说明: 现代依赖需 rustc >= 1.85。系统自带的 1.75 不可用;脚本会优先用 rustup 的
#       ~/.cargo/bin/cargo。若未安装 rustup, 见下方提示。

set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$REPO_DIR"

# ---- 选择 cargo ----
if [[ -x "$HOME/.cargo/bin/cargo" ]]; then
    CARGO="$HOME/.cargo/bin/cargo"
elif command -v cargo >/dev/null 2>&1; then
    CARGO="$(command -v cargo)"
else
    CARGO=""
fi

require_cargo() {
    if [[ -z "$CARGO" ]]; then
        cat >&2 <<'EOF'
错误: 未找到 cargo。请安装 rustup + 最新 stable:
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable -c clippy -c rustfmt
然后重开终端 (或 source "$HOME/.cargo/env") 再运行本脚本。
EOF
        exit 1
    fi
    # 版本校验 (>= 1.85)
    local ver major minor
    ver="$("$CARGO" --version 2>/dev/null | awk '{print $2}')"
    major="${ver%%.*}"; minor="$(echo "$ver" | cut -d. -f2)"
    if (( major < 1 || (major == 1 && minor < 85) )); then
        echo "错误: cargo $ver 过旧 (需 >= 1.85)。请用 rustup 安装新版 stable。" >&2
        echo "  当前: $CARGO" >&2
        exit 1
    fi
    echo ">> 使用 $CARGO ($ver)"
}

BIN="pbh"
PKG="pbh-server"

cmd="${1:-build}"
case "$cmd" in
    build)
        require_cargo
        echo ">> 发布构建 (release)…"
        "$CARGO" build --release -p "$PKG"
        out="target/release/$BIN"
        echo ">> 完成: $REPO_DIR/$out ($(du -h "$out" | cut -f1))"
        echo "   运行: PBH_DATA_DIR=./data $out   然后浏览器开 http://127.0.0.1:9898"
        ;;
    debug)
        require_cargo
        "$CARGO" build -p "$PKG"
        echo ">> 完成: target/debug/$BIN"
        ;;
    run)
        require_cargo
        "$CARGO" build -p "$PKG"
        echo ">> 运行 (Ctrl-C 退出)…"
        PBH_DATA_DIR="${PBH_DATA_DIR:-./data}" "target/debug/$BIN"
        ;;
    test)
        require_cargo
        "$CARGO" test --workspace
        ;;
    clippy)
        require_cargo
        "$CARGO" clippy --workspace --all-targets -- -D warnings
        "$CARGO" fmt --check
        echo ">> clippy + fmt 通过"
        ;;
    package)
        require_cargo
        echo ">> 发布构建 + 打包…"
        "$CARGO" build --release -p "$PKG"
        ver="$(grep -m1 '^version' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')"
        os="$(uname -s | tr '[:upper:]' '[:lower:]')"
        arch="$(uname -m)"
        name="pbh-rust-${ver}-${os}-${arch}"
        stage="dist/$name"
        rm -rf "$stage"; mkdir -p "$stage"
        cp "target/release/$BIN" "$stage/pbh-rust"
        cp README.md "$stage/" 2>/dev/null || true
        cat > "$stage/downloaders.yml.example" <<'YAML'
# 复制为 <数据目录>/config/downloaders.yml 并按需修改
- id: qb1
  type: qbittorrent        # 或 qbittorrentee
  name: 我的 qB
  endpoint: http://127.0.0.1:8080
  username: admin
  password: adminadmin
  increment-ban: false
  use-shadow-ban: false
  verify-ssl: true
  ignore-private: false
YAML
        cat > "$stage/运行说明.txt" <<'TXT'
PeerBanHelper-Rust 运行说明
1) 运行:  PBH_DATA_DIR=./data ./pbh-rust
2) 首次启动会在 ./data/ 生成配置并在日志打印一次 API token
3) 浏览器打开 http://127.0.0.1:9898 , 用该 token 登录
4) 在「下载器」里添加你的 qBittorrent, 即开始每 5 秒一轮自动封禁
TXT
        tar -C dist -czf "dist/$name.tar.gz" "$name"
        echo ">> 打包完成: $REPO_DIR/dist/$name.tar.gz ($(du -h "dist/$name.tar.gz" | cut -f1))"
        ;;
    clean)
        rm -rf target dist
        echo ">> 已清理 target/ 与 dist/"
        ;;
    *)
        echo "未知命令: $cmd" >&2
        sed -n '3,16p' "$0"
        exit 1
        ;;
esac
