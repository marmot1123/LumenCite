#!/usr/bin/env bash
#
# Linux 配布物に libpdfium.so が「実行時に探す場所」で同梱されているかを、成果物を展開して
# 静的に検査する（v1.0.0-p0 の残作業・docs/LCIR_REMAINING_PHASES.md §2.6-5 の (i)）。
#
# 同梱が落ちても本体は起動できてしまい、ユーザーが PDF を開いた瞬間に初めて
# 「pdfium library not found」になる。ビルドの側で止めないと気づけないので、
# リリースの Linux ジョブから毎回呼ぶ。
#
#   usage: scripts/verify_linux_bundle.sh [bundle-dir]
#     bundle-dir 既定 = src-tauri/target/release/bundle
#
# 期待する配置は Rust 側の探索候補（src-tauri/src/ingestion/pdf/pdfium.rs の
# `library_search_dirs`）と対でなければ意味が無い。実行ファイルが usr/bin/<bin> にあるとき
# 候補になる lib ディレクトリは `usr/lib/<name>` で、<name> は productName / crate 名 /
# 実行ファイル名の 3 通り。**片方だけ変えると検査が通ったまま実機で落ちる**ので、
# どちらかを触ったら必ずもう一方も見直すこと。
#
# .deb は macOS でも検査できる（ar + tar）。.AppImage は自己展開 ELF なので Linux が要る。
set -euo pipefail

BUNDLE_DIR="${1:-src-tauri/target/release/bundle}"

# productName（tauri.conf.json）と crate 名（src-tauri/Cargo.toml）。
PRODUCT_NAME="LumenCite"
CRATE_NAME="lumencite"

fail() {
  echo "::error::$*" >&2
  exit 1
}

# 成果物 1 つぶんの検査。$1 = 表示名, $2 = 中身のパス一覧（1 行 1 パス・先頭の ./ は除去済み）
check_listing() {
  local label="$1" listing="$2"

  if [ -z "$listing" ]; then
    fail "$label: 中身を 1 エントリも読み出せなかった（展開に失敗している）"
  fi

  # 実行ファイル名を成果物から読む（決め打ちしない ── mainBinaryName は設定で変わりうる）。
  local bins
  bins="$(printf '%s\n' "$listing" | grep -E '^usr/bin/[^/]+$' || true)"
  [ -n "$bins" ] || fail "$label: usr/bin/ に実行ファイルが無い"

  local so_paths
  so_paths="$(printf '%s\n' "$listing" | grep -E '(^|/)libpdfium\.so$' || true)"
  if [ -z "$so_paths" ]; then
    echo "--- $label の usr/lib 配下 ---" >&2
    printf '%s\n' "$listing" | grep -E '^usr/lib/' >&2 || echo "(なし)" >&2
    fail "$label: libpdfium.so が同梱されていない"
  fi

  # 候補ディレクトリを組む: productName / crate 名 / 実行ファイル名。
  local candidates=("usr/lib/$PRODUCT_NAME" "usr/lib/$CRATE_NAME")
  local b
  while IFS= read -r b; do
    candidates+=("usr/lib/$(basename "$b")")
  done <<<"$bins"

  local matched=""
  local so cand
  while IFS= read -r so; do
    for cand in "${candidates[@]}"; do
      if [ "$so" = "$cand/libpdfium.so" ]; then
        matched="$so"
        break 2
      fi
    done
  done <<<"$so_paths"

  if [ -z "$matched" ]; then
    echo "--- $label: 見つかった libpdfium.so ---" >&2
    printf '%s\n' "$so_paths" >&2
    echo "--- 実行時に探す候補 ---" >&2
    printf '%s/libpdfium.so\n' "${candidates[@]}" >&2
    fail "$label: libpdfium.so はあるが bind_pdfium() が探さない場所にある"
  fi

  echo "OK  $label: $matched"
}

# ── .deb ──────────────────────────────────────────────────────────────
check_deb() {
  local deb="$1" listing
  if command -v dpkg-deb >/dev/null 2>&1; then
    # Linux（CI の本番経路）。中身の tar を直接吐かせるのが一番素直で、
    # `dpkg-deb -c` の書式（tar tv 相当）を切り出すよりパスに空白があっても壊れない。
    listing="$(dpkg-deb --fsys-tarfile "$deb" | tar tf - | sed 's#^\./##' | sed 's#/$##')"
  else
    # macOS などでローカルに検査するとき用。BSD ar でも dpkg が書いた .deb は読める。
    local work abs data
    work="$(mktemp -d)"
    # shellcheck disable=SC2064
    trap "rm -rf '$work'" RETURN
    # ar は展開先へ cd してから呼ぶので、相対パスのままだと外れる。
    abs="$(cd "$(dirname "$deb")" && pwd)/$(basename "$deb")"
    ( cd "$work" && ar x "$abs" ) || fail "$(basename "$deb"): ar x に失敗"
    data="$(find "$work" -maxdepth 1 -name 'data.tar.*' | head -n 1)"
    [ -n "$data" ] || fail "$(basename "$deb"): data.tar.* が無い"
    listing="$(tar tf "$data" | sed 's#^\./##' | sed 's#/$##')"
  fi
  check_listing "$(basename "$deb")" "$listing"
}

# ── .rpm ──────────────────────────────────────────────────────────────
# libarchive（bsdtar）が rpm を読める。無い環境では **skip せず失敗させる**
# ── 「検査したが問題なし」と「検査していない」が同じ緑になるのを避ける。
check_rpm() {
  local rpm="$1"
  command -v bsdtar >/dev/null 2>&1 \
    || fail "$(basename "$rpm"): bsdtar が無く .rpm を検査できない（apt install libarchive-tools）"
  local listing
  listing="$(bsdtar tf "$rpm" | sed 's#^\./##' | sed 's#/$##')"
  check_listing "$(basename "$rpm")" "$listing"
}

# ── .AppImage ─────────────────────────────────────────────────────────
# 自己展開 ELF なので実行できる OS（= Linux x86_64）が要る。
check_appimage() {
  local img="$1" work
  work="$(mktemp -d)"
  # shellcheck disable=SC2064
  trap "rm -rf '$work'" RETURN
  local abs
  abs="$(cd "$(dirname "$img")" && pwd)/$(basename "$img")"
  chmod +x "$abs"
  ( cd "$work" && "$abs" --appimage-extract >/dev/null ) \
    || fail "$(basename "$img"): --appimage-extract に失敗（Linux 上で実行しているか）"
  [ -d "$work/squashfs-root" ] || fail "$(basename "$img"): squashfs-root が出てこない"
  local listing
  listing="$(cd "$work/squashfs-root" && find . -mindepth 1 | sed 's#^\./##')"
  check_listing "$(basename "$img")" "$listing"
}

# ── 走査 ──────────────────────────────────────────────────────────────
[ -d "$BUNDLE_DIR" ] || fail "バンドルディレクトリが無い: $BUNDLE_DIR"

checked=0
while IFS= read -r f; do
  case "$f" in
    *.deb)      check_deb "$f" ;;
    *.rpm)      check_rpm "$f" ;;
    *.AppImage) check_appimage "$f" ;;
    *)          continue ;;
  esac
  checked=$((checked + 1))
done < <(find "$BUNDLE_DIR" -type f \( -name '*.deb' -o -name '*.rpm' -o -name '*.AppImage' \) | sort)

# 1 つも見つからなければ「全部 OK」ではなく失敗にする。**成果物名やディレクトリ構成が
# 変わったときに、この検査だけが無言で空回りするのを防ぐ**（緑は「検査した」の意味であること）。
[ "$checked" -gt 0 ] || fail "$BUNDLE_DIR に Linux 成果物（.deb / .rpm / .AppImage）が 1 つも無い"

echo "verify_linux_bundle: $checked 個の成果物すべてに libpdfium.so を確認した"
