#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: create-app-bundle.sh --binary PATH --version SEMVER --output PATH.app [--build-version NUMBER]

Creates a macOS PaperlessLLM.app bundle without modifying the source binary.
On macOS, the completed bundle receives a valid ad-hoc signature. Release
automation may replace it with a Developer ID signature afterward.
The output path must not already exist.
EOF
}

fail() {
  printf 'create-app-bundle: %s\n' "$1" >&2
  exit 1
}

binary=""
version=""
build_version=""
output=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --binary)
      [[ $# -ge 2 ]] || fail "--binary requires a value"
      binary=$2
      shift 2
      ;;
    --version)
      [[ $# -ge 2 ]] || fail "--version requires a value"
      version=$2
      shift 2
      ;;
    --build-version)
      [[ $# -ge 2 ]] || fail "--build-version requires a value"
      build_version=$2
      shift 2
      ;;
    --output)
      [[ $# -ge 2 ]] || fail "--output requires a value"
      output=$2
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage
      fail "unknown argument: $1"
      ;;
  esac
done

[[ -n "$binary" ]] || fail "--binary is required"
[[ -f "$binary" && -x "$binary" ]] || fail "binary is not an executable file: $binary"
[[ -n "$version" ]] || fail "--version is required"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$ ]] \
  || fail "version must be semantic version text without a leading v"
[[ -n "$output" ]] || fail "--output is required"
[[ "$output" == *.app ]] || fail "output path must end in .app"

marketing_version=${version%%[-+]*}
if [[ -z "$build_version" ]]; then
  build_version=$marketing_version
fi
[[ "$build_version" =~ ^[0-9]+(\.[0-9]+){0,2}$ ]] \
  || fail "build version must contain one to three numeric components"

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
template="$script_dir/Info.plist.in"
[[ -f "$template" ]] || fail "Info.plist template is missing: $template"

mkdir -p "$(dirname "$output")"
output_parent=$(cd "$(dirname "$output")" && pwd -P)
output_name=$(basename "$output")
output="$output_parent/$output_name"
[[ ! -e "$output" ]] || fail "output already exists: $output"

staging=$(mktemp -d "$output_parent/.paperlessllm-app.XXXXXX")
cleanup() {
  rm -rf "$staging"
}
trap cleanup EXIT

bundle="$staging/$output_name"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
install -m 0755 "$binary" "$bundle/Contents/MacOS/paperless-llm"
sed \
  -e "s|__FULL_VERSION__|$version|g" \
  -e "s|__MARKETING_VERSION__|$marketing_version|g" \
  -e "s|__BUILD_VERSION__|$build_version|g" \
  "$template" >"$bundle/Contents/Info.plist"

if command -v plutil >/dev/null 2>&1; then
  plutil -lint "$bundle/Contents/Info.plist" >/dev/null
fi

if [[ "$(uname -s)" == "Darwin" ]]; then
  command -v codesign >/dev/null 2>&1 || fail "codesign is required on macOS"
  bundle_binary="$bundle/Contents/MacOS/paperless-llm"
  codesign --force --sign - "$bundle_binary"
  codesign --force --sign - "$bundle"
  codesign --verify --strict --verbose=2 "$bundle_binary"
  codesign --verify --deep --strict --verbose=2 "$bundle"
fi

mv "$bundle" "$output"
rmdir "$staging"
trap - EXIT

printf '%s\n' "$output"
