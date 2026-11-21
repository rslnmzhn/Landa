#!/usr/bin/env bash
set -euo pipefail

VERSION=""
VERSION_CODE=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version)
      VERSION="$2"
      shift 2
      ;;
    --version-code)
      VERSION_CODE="$2"
      shift 2
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

if [[ -z "${VERSION}" || -z "${VERSION_CODE}" ]]; then
  echo "Usage: apply_release_version.sh --version X.Y.Z --version-code N" >&2
  exit 1
fi

python3 - "${VERSION}" "${VERSION_CODE}" <<'PY'
import json
import pathlib
import re
import sys

version = sys.argv[1]
version_code = sys.argv[2]

# 1. package.json
pkg_path = pathlib.Path("package.json")
pkg = json.loads(pkg_path.read_text(encoding="utf-8"))
pkg["version"] = version
pkg_path.write_text(json.dumps(pkg, indent=2) + "\n", encoding="utf-8")

# 2. src-tauri/tauri.conf.json
conf_path = pathlib.Path("src-tauri/tauri.conf.json")
conf = json.loads(conf_path.read_text(encoding="utf-8"))
conf["version"] = version
conf_path.write_text(json.dumps(conf, indent=2) + "\n", encoding="utf-8")

# 3. Cargo.toml & src-tauri/Cargo.toml
for cargo_file in [pathlib.Path("Cargo.toml"), pathlib.Path("src-tauri/Cargo.toml")]:
    text = cargo_file.read_text(encoding="utf-8")
    updated = re.sub(
        r'(?m)^version\s*=\s*"[^"]+"',
        f'version = "{version}"',
        text,
        count=1,
    )
    cargo_file.write_text(updated, encoding="utf-8")

# 4. src-tauri/gen/android/app/tauri.properties
props_path = pathlib.Path("src-tauri/gen/android/app/tauri.properties")
props_path.parent.mkdir(parents=True, exist_ok=True)
props_path.write_text(
    f"tauri.android.versionCode={version_code}\n"
    f"tauri.android.versionName={version}\n",
    encoding="utf-8",
)
PY

echo "Applied release version ${VERSION} (versionCode=${VERSION_CODE})"
