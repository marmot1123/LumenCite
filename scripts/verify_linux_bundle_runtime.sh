#!/usr/bin/env bash
#
# Linux の実配布物の中で `bind_pdfium()` が**実際に**同梱ライブラリを掴むかを確かめる
# （v1.0.0-p0・docs/LCIR_REMAINING_PHASES.md §2.6-5 の (ii)）。
# 静的検査（verify_linux_bundle.sh）が答えられない「探索候補と実配置が噛み合っているか」を見る。
#
#   usage: scripts/verify_linux_bundle_runtime.sh <probe-binary> [bundle-dir]
#     probe-binary  cargo build --release --example pdfium_probe の成果物
#     bundle-dir    既定 = src-tauri/target/release/bundle
#
# ⚠ **破壊的**: .deb を実際にインストールし、**インストール済み本体バイナリを一時的に
# プローブへ置き換える**。終了時に必ず戻すが（EXIT トラップ）、常用の Linux 機では回さないこと。
# 既定では CI 以外で走らない。手元で敢えて回すなら LUMENCITE_ALLOW_DESTRUCTIVE=1 を付ける。
#
# **Linux 専用**（.deb を dpkg で入れ、.AppImage を自己展開させる）。
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

fail() { echo "::error::$*" >&2; exit 1; }

if [ "${CI:-}" != "true" ] && [ "${LUMENCITE_ALLOW_DESTRUCTIVE:-}" != "1" ]; then
  fail "このスクリプトは .deb をインストールし本体バイナリを一時的に置き換えます。使い捨て環境でのみ実行してください（続行するなら LUMENCITE_ALLOW_DESTRUCTIVE=1）"
fi

# `dirname` が存在しないパスでも代入自体は成功する（末尾の basename が 0 を返すため）ので、
# ここで落とさず下の -x 判定に落とす。
PROBE="$(cd "$(dirname "$PROBE")" 2>/dev/null && pwd || true)/$(basename "$PROBE")"
[ -x "$PROBE" ] || { echo "::error::プローブが無い / 実行権が無い: $PROBE" >&2; exit 1; }

# 後始末は EXIT にまとめる。**`fail` は exit なので RETURN トラップでは発火しない** ──
# 一番まずいのは、置き換えた本体バイナリが戻らないまま抜けること。戻さないと
# 2 回目の実行が「既にプローブになっている本体」を .real へ退避し、**本物を上書きして消す**。
RESTORE_EXE=""
RESTORE_SO=""
WORKDIRS=()
cleanup() {
  if [ -n "$RESTORE_SO" ] && [ -e "$RESTORE_SO.hidden" ]; then
    sudo mv -f "$RESTORE_SO.hidden" "$RESTORE_SO" 2>/dev/null || true
  fi
  if [ -n "$RESTORE_EXE" ] && [ -e "$RESTORE_EXE.real" ]; then
    sudo mv -f "$RESTORE_EXE.real" "$RESTORE_EXE" 2>/dev/null || true
  fi
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

abs_path() {
  printf '%s' "$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
}

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
  local label="$1" exe_path="$2" so_path="$3" loaded
  echo "── $label"
  echo "   本体バイナリ: $exe_path"
  echo "   同梱 pdfium : $so_path"

  # 本体を退けてプローブを同じパスへ。EXIT トラップが必ず戻す。
  sudo mv "$exe_path" "$exe_path.real"
  RESTORE_EXE="$exe_path"
  sudo cp "$PROBE" "$exe_path"

  # (1) 成功すること
  if ! run_probe "$exe_path"; then
    printf '%s\n' "$PROBE_OUT" >&2
    fail "$label: bind_pdfium() が同梱ライブラリを見つけられなかった"
  fi
  printf '%s\n' "$PROBE_OUT" | sed 's/^/   /'

  # (2) 掴んだのが**この配布物の .so** であること。
  # `<unknown>`（/proc が読めない等で実パスを取れなかった）は「検証できなかった」＝失敗。
  loaded="$(printf '%s\n' "$PROBE_OUT" | sed -n 's/^probe: loaded=//p' | head -n 1)"
  [ -n "$loaded" ] || fail "$label: dlopen された実パスの行が出ていない"
  [ "$loaded" != "<unknown>" ] || fail "$label: dlopen された実パスを取得できなかった（/proc が読めない？）"
  [ "$loaded" = "$so_path" ] \
    || fail "$label: 同梱ぶんではなく別の pdfium を掴んだ（loaded=$loaded / 期待=${so_path}）"

  # (3) 退かすと失敗すること（(1) がシステムライブラリ等の別経路でないことの裏取り）
  sudo mv "$so_path" "$so_path.hidden"
  RESTORE_SO="$so_path"
  if run_probe "$exe_path"; then
    printf '%s\n' "$PROBE_OUT" >&2
    fail "$label: 同梱ぶんを退けても bind に成功した ── この機械では対照実験が成立していない（システムに libpdfium がある）"
  fi
  echo "   対照: 同梱ぶんを退けると想定どおり失敗した"
  sudo mv -f "$so_path.hidden" "$so_path"
  RESTORE_SO=""

  sudo mv -f "$exe_path.real" "$exe_path"
  RESTORE_EXE=""
  echo "OK $label"
}

[ -d "$BUNDLE_DIR" ] || fail "バンドルディレクトリが無い: $BUNDLE_DIR"
checked=0

# ── .deb: 実際にインストールして /usr の実配置で試す ───────────────────
deb="$(find "$BUNDLE_DIR" -type f -name '*.deb' | sort | head -n 1)"
if [ -n "$deb" ]; then
  deb="$(abs_path "$deb")"
  sudo apt-get install -y "$deb"
  pkg="$(dpkg-deb -f "$deb" Package)"
  # `|| true` が要る: grep が 1 件も当たらないと pipefail + set -e が**代入の時点で**
  # スクリプトを殺し、直後の fail の文言（= どの検査で落ちたか）が永久に出ない。
  exe="$(dpkg -L "$pkg" | grep -E '^/usr/bin/[^/]+$' | head -n 1 || true)"
  [ -n "$exe" ] || fail "$(basename "$deb"): インストール後に /usr/bin の実行ファイルが見つからない"
  so="$(dpkg -L "$pkg" | grep -E '/libpdfium\.so$' | head -n 1 || true)"
  [ -n "$so" ] || fail "$(basename "$deb"): インストール後に libpdfium.so が見つからない"
  verify_layout "$(basename "$deb")" "$exe" "$so"
  checked=$((checked + 1))
fi

# ── .AppImage: 自己展開した $APPDIR 相当のツリーで試す ─────────────────
img="$(find "$BUNDLE_DIR" -type f -name '*.AppImage' | sort | head -n 1)"
if [ -n "$img" ]; then
  img="$(abs_path "$img")"
  work="$(new_workdir)"
  chmod +x "$img"
  ( cd "$work" && "$img" --appimage-extract >/dev/null ) \
    || fail "$(basename "$img"): --appimage-extract に失敗"
  root="$work/squashfs-root"
  # find はディレクトリ不在で非 0 を返すので、ここも `|| true` が無いと fail に届かない。
  exe="$(find "$root/usr/bin" -maxdepth 1 -type f 2>/dev/null | sort | head -n 1 || true)"
  [ -n "$exe" ] || fail "$(basename "$img"): usr/bin に実行ファイルが無い"
  so="$(find "$root" -name 'libpdfium.so' | sort | head -n 1 || true)"
  [ -n "$so" ] || fail "$(basename "$img"): libpdfium.so が同梱されていない"
  verify_layout "$(basename "$img")" "$exe" "$so"
  checked=$((checked + 1))
fi

# 対象が 1 つも無いまま緑にしない（成果物名やディレクトリ構成が変わったときに空回りするのを防ぐ）。
[ "$checked" -ge 2 ] \
  || fail "検証できた配布形態が $checked 件しかない（.deb と .AppImage の両方が要る・§2.6-5）"

echo "verify_linux_bundle_runtime: .deb と .AppImage の両方で bind_pdfium() が同梱ぶんを掴んだ"
