#!/usr/bin/env bash
#
# Linux 配布物に libpdfium.so が「実行時に探す場所」で同梱されているかを、成果物を展開して
# 静的に検査する（v1.0.0-p0・docs/LCIR_REMAINING_PHASES.md §2.6-5 の (i)）。
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
# **この検査が答えないこと**: 実行時に `bind_pdfium()` がその .so を実際に掴むか。
# それは scripts/verify_linux_bundle_runtime.sh（.deb を入れて中で呼ぶ）の担当で、
# 走るのは linux-bundle-verify ワークフロー。リリース経路が毎回回すのはこちらだけなので、
# **タグを打つ前に一度は実行時検証も回すこと**（RELEASE.md §5）。
#
# .deb は macOS でも検査できる（ar + tar）。.AppImage は自己展開 ELF なので Linux が要る。
set -euo pipefail

BUNDLE_DIR="${1:-src-tauri/target/release/bundle}"

# productName（tauri.conf.json）と crate 名（src-tauri/Cargo.toml）。
PRODUCT_NAME="LumenCite"
CRATE_NAME="lumencite"

# 後始末は EXIT にまとめる。**`fail` は exit なので RETURN トラップでは発火せず**、
# 主要な失敗経路（= 検査が落ちたとき）でだけ展開物が残る、という一番いらない挙動になる。
WORKDIRS=()
cleanup() {
  local d
  for d in ${WORKDIRS[@]+"${WORKDIRS[@]}"}; do
    [ -n "$d" ] && rm -rf "$d"
  done
}
trap cleanup EXIT

new_workdir() {
  local d
  d="$(mktemp -d)"
  WORKDIRS+=("$d")
  printf '%s' "$d"
}

fail() {
  echo "::error::$*" >&2
  exit 1
}

abs_path() {
  printf '%s' "$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
}

# 同梱された .so が**本当にロードできる形**か。名前だけの一致で緑にしない
# （空ファイルでも、うっかり別アーキテクチャのものでも、エントリ名は同じになる）。
# `file` / `readelf` に頼らず ELF ヘッダを直接読むので、macOS で検査しても同じ判定になる。
assert_elf_shared_object() {
  local f="$1" label="$2" size magic etype emach
  [ -f "$f" ] || fail "$label: $f が通常ファイルではない（シンボリックリンク等）"
  size="$(wc -c <"$f" | tr -d ' ')"
  [ "$size" -gt 0 ] || fail "$label: libpdfium.so が空ファイル"
  magic="$(od -An -tx1 -N4 "$f" | tr -d ' \n')"
  [ "$magic" = "7f454c46" ] || fail "$label: libpdfium.so が ELF ではない（先頭 4 バイト = ${magic}）"
  # e_type (offset 16) / e_machine (offset 18) はどちらも 2 バイト。ELF64 little-endian を
  # 前提に od のネイティブ 2 バイト読みで比べる（検査する機械も x86-64 / arm64 = LE）。
  etype="$(od -An -tu2 -j16 -N2 "$f" | tr -d ' \n')"
  [ "$etype" = "3" ] || fail "$label: libpdfium.so が共有オブジェクトでない（e_type = ${etype}・3 = ET_DYN）"
  emach="$(od -An -tu2 -j18 -N2 "$f" | tr -d ' \n')"
  [ "$emach" = "62" ] || fail "$label: libpdfium.so が x86-64 向けでない（e_machine = ${emach}・62 = EM_X86_64）"
  echo "    ELF 共有オブジェクト / x86-64 / ${size} bytes"
}

# 展開済みツリー 1 つぶんの検査。$1 = 表示名, $2 = ツリーの根（この下に usr/ が来る）
check_tree() {
  local label="$1" root="$2" listing bins so_paths matched so cand b

  listing="$(cd "$root" && find . -mindepth 1 | sed 's#^\./##' | sort)"
  [ -n "$listing" ] || fail "$label: 中身を 1 エントリも読み出せなかった（展開に失敗している）"

  # 実行ファイル名を成果物から読む（決め打ちしない ── mainBinaryName は設定で変わりうる）。
  bins="$(printf '%s\n' "$listing" | grep -E '^usr/bin/[^/]+$' || true)"
  [ -n "$bins" ] || fail "$label: usr/bin/ に実行ファイルが無い"

  so_paths="$(printf '%s\n' "$listing" | grep -E '(^|/)libpdfium\.so$' || true)"
  if [ -z "$so_paths" ]; then
    echo "--- $label の usr/lib 配下 ---" >&2
    printf '%s\n' "$listing" | grep -E '^usr/lib/' >&2 || echo "(なし)" >&2
    fail "$label: libpdfium.so が同梱されていない"
  fi

  # 候補ディレクトリを組む: productName / crate 名 / 実行ファイル名。
  local candidates=("usr/lib/$PRODUCT_NAME" "usr/lib/$CRATE_NAME")
  while IFS= read -r b; do
    candidates+=("usr/lib/$(basename "$b")")
  done <<<"$bins"

  matched=""
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
  assert_elf_shared_object "$root/$matched" "$label"
}

# ── .deb ──────────────────────────────────────────────────────────────
check_deb() {
  local deb work
  deb="$(abs_path "$1")"
  work="$(new_workdir)"
  if command -v dpkg-deb >/dev/null 2>&1; then
    # Linux（CI の本番経路）。中身の tar を直接吐かせるのが一番素直。
    dpkg-deb --fsys-tarfile "$deb" | tar xf - -C "$work" \
      || fail "$(basename "$deb"): dpkg-deb での展開に失敗"
  else
    # macOS などでローカルに検査するとき用。BSD ar でも dpkg が書いた .deb は読める。
    local ar_dir data
    ar_dir="$(new_workdir)"
    ( cd "$ar_dir" && ar x "$deb" ) || fail "$(basename "$deb"): ar x に失敗"
    data="$(find "$ar_dir" -maxdepth 1 -name 'data.tar.*' | head -n 1)"
    [ -n "$data" ] || fail "$(basename "$deb"): data.tar.* が無い"
    tar xf "$data" -C "$work" || fail "$(basename "$deb"): data.tar.* の展開に失敗"
  fi
  check_tree "$(basename "$deb")" "$work"
}

# ── .rpm ──────────────────────────────────────────────────────────────
# libarchive（bsdtar）が rpm を読める。無い環境では **skip せず失敗させる**
# ── 「検査したが問題なし」と「検査していない」が同じ緑になるのを避ける。
check_rpm() {
  local rpm work
  rpm="$(abs_path "$1")"
  command -v bsdtar >/dev/null 2>&1 \
    || fail "$(basename "$rpm"): bsdtar が無く .rpm を検査できない（apt install libarchive-tools）"
  work="$(new_workdir)"
  bsdtar xf "$rpm" -C "$work" || fail "$(basename "$rpm"): bsdtar での展開に失敗"
  check_tree "$(basename "$rpm")" "$work"
}

# ── .AppImage ─────────────────────────────────────────────────────────
# 自己展開 ELF なので実行できる OS（= Linux x86_64）が要る。
check_appimage() {
  local img work
  img="$(abs_path "$1")"
  work="$(new_workdir)"
  chmod +x "$img"
  ( cd "$work" && "$img" --appimage-extract >/dev/null ) \
    || fail "$(basename "$img"): --appimage-extract に失敗（Linux 上で実行しているか）"
  [ -d "$work/squashfs-root" ] || fail "$(basename "$img"): squashfs-root が出てこない"
  check_tree "$(basename "$img")" "$work/squashfs-root"
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
