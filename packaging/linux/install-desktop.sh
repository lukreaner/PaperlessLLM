#!/bin/sh

set -eu

application_id=io.paperlessllm.PaperlessLLM
script_dir=$(CDPATH= cd -P "$(dirname "$0")" && pwd)
release_dir=$(CDPATH= cd -P "$script_dir/../.." && pwd)
source_binary=$release_dir/paperless-llm
desktop_template=$script_dir/$application_id.desktop.in
create_desktop_shortcut=false

usage() {
  cat <<'EOF'
Usage: ./packaging/linux/install-desktop.sh [--desktop-shortcut]

Installs PaperlessLLM for the current user and adds it to the application menu.
The optional flag also creates an executable PaperlessLLM.desktop shortcut in
the user's desktop directory.

Optional environment overrides:
  PAPERLESSLLM_BIN_DIR       Binary destination (default: $HOME/.local/bin)
  PAPERLESSLLM_DESKTOP_DIR   Desktop shortcut destination directory
  XDG_DATA_HOME              XDG user data directory
EOF
}

fail() {
  printf 'PaperlessLLM installer: %s\n' "$*" >&2
  exit 1
}

require_absolute_path() {
  variable_name=$1
  path_value=$2
  case $path_value in
    /*) ;;
    *) fail "$variable_name must be an absolute path" ;;
  esac
}

escape_desktop_exec_argument() {
  # Desktop Entry Exec quoting requires these characters to be backslash
  # escaped inside double quotes. A doubled percent is a literal percent rather
  # than a field code.
  LC_ALL=C sed \
    -e 's/\\/\\\\/g' \
    -e 's/"/\\"/g' \
    -e 's/`/\\`/g' \
    -e 's/\$/\\$/g' \
    -e 's/%/%%/g'
}

while [ "$#" -gt 0 ]; do
  case $1 in
    --desktop-shortcut)
      create_desktop_shortcut=true
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage >&2
      fail "unknown option: $1"
      ;;
  esac
  shift
done

[ -n "${HOME:-}" ] || fail 'HOME is not set'
require_absolute_path HOME "$HOME"

data_home=${XDG_DATA_HOME:-$HOME/.local/share}
bin_dir=${PAPERLESSLLM_BIN_DIR:-$HOME/.local/bin}
applications_dir=$data_home/applications
data_dir=$data_home/paperless-llm
binary_destination=$bin_dir/paperless-llm
desktop_destination=$applications_dir/$application_id.desktop

require_absolute_path XDG_DATA_HOME "$data_home"
require_absolute_path PAPERLESSLLM_BIN_DIR "$bin_dir"

[ -f "$source_binary" ] || fail "release binary not found at $source_binary"
[ -x "$source_binary" ] || fail "release binary is not executable: $source_binary"
[ -f "$desktop_template" ] || fail "desktop template not found at $desktop_template"

case $binary_destination in
  *'='*) fail 'the binary destination contains an equals sign unsupported by Desktop Entry Exec' ;;
esac
binary_path_lines=$(printf '%s\n' "$binary_destination" | wc -l)
[ "$binary_path_lines" -eq 1 ] || fail 'the binary destination contains a newline unsupported by Desktop Entry Exec'

mkdir -p "$bin_dir" "$applications_dir" "$data_dir"
chmod 0700 "$data_dir"

binary_tmp=
desktop_tmp=
shortcut_tmp=
cleanup() {
  [ -z "$binary_tmp" ] || rm -f "$binary_tmp"
  [ -z "$desktop_tmp" ] || rm -f "$desktop_tmp"
  [ -z "$shortcut_tmp" ] || rm -f "$shortcut_tmp"
}
trap cleanup 0 HUP INT TERM

binary_tmp=$(mktemp "$bin_dir/.paperless-llm.XXXXXX")
cp "$source_binary" "$binary_tmp"
chmod 0755 "$binary_tmp"
mv -f "$binary_tmp" "$binary_destination"
binary_tmp=

escaped_binary=$(printf '%s' "$binary_destination" | escape_desktop_exec_argument)
desktop_tmp=$(mktemp "$applications_dir/.$application_id.desktop.XXXXXX")
placeholder_count=0
while IFS= read -r line || [ -n "$line" ]; do
  if [ "$line" = 'Exec=__PAPERLESS_LLM_EXEC__ launch' ]; then
    printf 'Exec="%s" launch\n' "$escaped_binary"
    placeholder_count=$((placeholder_count + 1))
  else
    printf '%s\n' "$line"
  fi
done <"$desktop_template" >"$desktop_tmp"
[ "$placeholder_count" -eq 1 ] || fail 'desktop template does not contain exactly one Exec placeholder'
chmod 0644 "$desktop_tmp"

if command -v desktop-file-validate >/dev/null 2>&1; then
  desktop-file-validate "$desktop_tmp"
fi

mv -f "$desktop_tmp" "$desktop_destination"
desktop_tmp=

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$applications_dir" >/dev/null 2>&1 || true
fi

desktop_shortcut=
if [ "$create_desktop_shortcut" = true ]; then
  if [ -n "${PAPERLESSLLM_DESKTOP_DIR:-}" ]; then
    desktop_dir=$PAPERLESSLLM_DESKTOP_DIR
  elif command -v xdg-user-dir >/dev/null 2>&1; then
    desktop_dir=$(xdg-user-dir DESKTOP 2>/dev/null || true)
    [ -n "$desktop_dir" ] || desktop_dir=$HOME/Desktop
  else
    desktop_dir=$HOME/Desktop
  fi
  require_absolute_path PAPERLESSLLM_DESKTOP_DIR "$desktop_dir"
  mkdir -p "$desktop_dir"
  desktop_shortcut=$desktop_dir/PaperlessLLM.desktop
  shortcut_tmp=$(mktemp "$desktop_dir/.PaperlessLLM.desktop.XXXXXX")
  cp "$desktop_destination" "$shortcut_tmp"
  chmod 0755 "$shortcut_tmp"
  mv -f "$shortcut_tmp" "$desktop_shortcut"
  shortcut_tmp=

  # GNOME-based file managers may use this metadata in addition to the
  # executable bit. Other desktops ignore it, and failure is harmless.
  if command -v gio >/dev/null 2>&1; then
    gio set "$desktop_shortcut" metadata::trusted true >/dev/null 2>&1 || true
  fi
fi

trap - 0 HUP INT TERM

printf 'PaperlessLLM installed for the current user.\n'
printf '  Binary: %s\n' "$binary_destination"
printf '  Application menu entry: %s\n' "$desktop_destination"
if [ -n "$desktop_shortcut" ]; then
  printf '  Desktop shortcut: %s\n' "$desktop_shortcut"
fi
printf 'Open PaperlessLLM from the application menu or run:\n  "%s" launch\n' "$binary_destination"
