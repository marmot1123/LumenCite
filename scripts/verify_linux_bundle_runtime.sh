#!/usr/bin/env bash
#
# Linux の実配布物の中で `bind_pdfium()` が**実際に**同梱ライブラリを掴むかを確かめる
# （v1.0.0-p0 の残作業・docs/LCIR_REMAINING_PHASES.md §2.6-5 の (ii)）。
# 静的検査（verify_linux_bundle.sh）が答えられない「探索候補と実配置が噛み合っているか」を見る。
#
#   usage: scripts/verify_linux_bundle_runtime.sh <probe-binary> [bundle-dir]
#     probe-binary  cargo build --release --example pdfium_probe の成果物
#     bundle-dir    既定 = src-tauri/target/release/bundle
#
# **Linux 専用**（.deb を実際にインストールし、.AppImage を自己展開させる）。
#
# 本体バイナリは webview を立ち上げてユーザーが PDF を触るまで pdfium に接触しないので、
# ヘッドレスな CI では問える瞬間が来ない。そこで本体と**同じパスに**プローブを置き換えて回す
# （`bind_pdfium()` の探索は `std::env::current_exe()` の置き場所だけに依存する）。
#
# 判定は 3 点セットで、どれ 1 つ欠けても「見つかった」の意味が薄まる:
#   (1) プローブが成功する
#   (2) dlopen された実パスが、**その配布物が置いた .so そのもの**である
#       （`bind_to_system_library()` のフォールバックで成功しても (2) で落ちる）
#   (3) その .so を退かすとプローブが失敗する（= (1) が別経路の当たりではない裏取り）
set -euo pipefail

PROBE="${1:?usage: $0 <probe-binary> [bundle-dir]}"
BUNDLE_DIR="${2:-src-tauri/target/release/bundle}"

PROBE="$(cd "$(dirname "$PROBE")" && pwd)/$(basename "$PROBE")"
[ -x "$PROBE" ] || { echo "::error::プローブが無い / 実行権が無い: $PROBE" >&2; exit 1; }

fail() { echo "::error::$*" >&2; exit 1; }

# プローブを**空のカレントディレクトリ**で回す。`library_search_dirs` の末尾には dev 用の
# `pdfium` と `.` があるので、リポジトリの中で回すとバンドル配置と無関係に当たりうる。
run_probe() {
  local exe="$1" d out
  d="$(mktemp -d)"
  if out="$(cd "$d" && "$exe" 2>&1)"; then
    PROBE_OUT="$out"; rm -rf "$d"; return 0
  else
    PROBE_OUT="$out"; rm -rf "$d"; return 1
  fi
}

# $1 = 表示名, $2 = プローブを置くパス（= 本体バイナリのパス）, $3 = その配布物が置いた .so の絶対パス
verify_layout() {
  local label="$1" exe_path="$2" so_path="$3"
  echo "── $label"
  echo "   本体バイナリ: $exe_path"
  echo "   同梱 pdfium : $so_path"

  # 本体を退けてプローブを同じパスへ。以後この配布物は起動できないが、使い捨てのランナー上の話。
  sudo mv "$exe_path" "$exe_path.real"
  sudo cp "$PROBE" "$exe_path"

  # (1) 成功すること
  if ! run_probe "$exe_path"; then
    printf '%s\n' "$PROBE_OUT" >&2
    fail "$label: bind_pdfium() が同梱ライブラリを見つけられなかった"
  fi
  printf '%s\n' "$PROBE_OUT" | sed 's/^/   /'

  # (2) 掴んだのが**この配布物の .so** であること
  local loaded
  loaded="$(printf '%s\n' "$PROBE_OUT" | sed -n 's/^probe: loaded=//p' | head -n 1)"
  [ -n "$loaded" ] || fail "$label: dlopen された実パスを取得できなかった（/proc が読めない？）"
  [ "$loaded" = "$so_path" ] \
    || fail "$label: 同梱ぶんではなく別の pdfium を掴んだ（loaded=$loaded / 期待=$so_path）"

  # (3) 退かすと失敗すること（(1) がシステムライブラリ等の別経路でないことの裏取り）
  sudo mv "$so_path" "$so_path.hidden"
  if run_probe "$exe_path"; then
    printf '%s\n' "$PROBE_OUT" >&2
    sudo mv "$so_path.hidden" "$so_path"
    fail "$label: 同梱ぶんを退けても bind に成功した ── この機械では対照実験が成立していない（システムに libpdfium がある）"
  fi
  echo "   対照: 同梱ぶんを退けると想定どおり失敗した"
  sudo mv "$so_path.hidden" "$so_path"

  echo "OK $label"
}

[ -d "$BUNDLE_DIR" ] || fail "バンドルディレクトリが無い: $BUNDLE_DIR"
checked=0

# ── .deb: 実際にインストールして /usr の実配置で試す ───────────────────
deb="$(find "$BUNDLE_DIR" -type f -name '*.deb' | sort | head -n 1)"
if [ -n "$deb" ]; then
  deb="$(cd "$(dirname "$deb")" && pwd)/$(basename "$deb")"
  sudo apt-get install -y "$deb"
  pkg="$(dpkg-deb -f "$deb" Package)"
  exe="$(dpkg -L "$pkg" | grep -E '^/usr/bin/[^/]+$' | head -n 1)"
  [ -n "$exe" ] || fail "$(basename "$deb"): インストール後に /usr/bin の実行ファイルが見つからない"
  so="$(dpkg -L "$pkg" | grep -E '/libpdfium\.so$' | head -n 1)"
  [ -n "$so" ] || fail "$(basename "$deb"): インストール後に libpdfium.so が見つからない"
  verify_layout "$(basename "$deb")" "$exe" "$so"
  checked=$((checked + 1))
fi

# ── .AppImage: 自己展開した $APPDIR 相当のツリーで試す ─────────────────
img="$(find "$BUNDLE_DIR" -type f -name '*.AppImage' | sort | head -n 1)"
if [ -n "$img" ]; then
  img="$(cd "$(dirname "$img")" && pwd)/$(basename "$img")"
  work="$(mktemp -d)"
  chmod +x "$img"
  ( cd "$work" && "$img" --appimage-extract >/dev/null ) \
    || fail "$(basename "$img"): --appimage-extract に失敗"
  root="$work/squashfs-root"
  exe="$(find "$root/usr/bin" -maxdepth 1 -type f | sort | head -n 1)"
  [ -n "$exe" ] || fail "$(basename "$img"): usr/bin に実行ファイルが無い"
  so="$(find "$root" -name 'libpdfium.so' | sort | head -n 1)"
  [ -n "$so" ] || fail "$(basename "$img"): libpdfium.so が同梱されていない"
  verify_layout "$(basename "$img")" "$exe" "$so"
  checked=$((checked + 1))
fi

# 対象が 1 つも無いまま緑にしない（成果物名やディレクトリ構成が変わったときに空回りするのを防ぐ）。
[ "$checked" -ge 2 ] \
  || fail "検証できた配布形態が $checked 件しかない（.deb と .AppImage の両方が要る・§2.6-5）"

echo "verify_linux_bundle_runtime: .deb と .AppImage の両方で bind_pdfium() が同梱ぶんを掴んだ"
