#!/usr/bin/env bash
set -euo pipefail

BASE_VERSION="$(
  python3 -c 'import json; print(json.load(open("src-tauri/tauri.conf.json", encoding="utf-8"))["version"])'
)"
if [[ -z "${BASE_VERSION}" ]]; then
  echo "Unable to read base version from src-tauri/tauri.conf.json" >&2
  exit 1
fi

git fetch --tags --force >/dev/null 2>&1 || true

LATEST_STABLE_VERSION="$(
  python3 - "${BASE_VERSION}" <<'PY'
import re
import subprocess
import sys

base = tuple(int(x) for x in sys.argv[1].split("."))
tags = subprocess.check_output(["git", "tag", "--list", "v*"], text=True).splitlines()
versions = []
for tag in tags:
    m = re.fullmatch(r"v(\d+)\.(\d+)\.(\d+)", tag.strip())
    if not m:
        continue
    ver = tuple(int(g) for g in m.groups())
    if ver >= base:
        versions.append(ver)

if versions:
    best = max(versions)
    print(f"{best[0]}.{best[1]}.{best[2]}")
PY
)"

if [[ -z "${LATEST_STABLE_VERSION}" ]]; then
  RELEASE_VERSION="${BASE_VERSION}"
else
  IFS='.' read -r MAJOR MINOR PATCH <<<"${LATEST_STABLE_VERSION}"
  PATCH=$((PATCH + 1))
  if (( PATCH > 9 )); then
    PATCH=0
    MINOR=$((MINOR + 1))
  fi
  if (( MINOR > 9 )); then
    MINOR=0
    MAJOR=$((MAJOR + 1))
  fi
  RELEASE_VERSION="${MAJOR}.${MINOR}.${PATCH}"
fi

IFS='.' read -r MAJOR MINOR PATCH <<<"${RELEASE_VERSION}"
for segment in "${MAJOR}" "${MINOR}" "${PATCH}"; do
  if ! [[ "${segment}" =~ ^[0-9]+$ ]]; then
    echo "Release version contains a non-numeric segment: ${RELEASE_VERSION}" >&2
    exit 1
  fi
done

ANDROID_VERSION_CODE=$((MAJOR * 1000000 + MINOR * 1000 + PATCH))
ANDROID_VERSION_CODE_LIMIT=2100000000
if (( ANDROID_VERSION_CODE <= 0 || ANDROID_VERSION_CODE > ANDROID_VERSION_CODE_LIMIT )); then
  echo "Computed Android versionCode ${ANDROID_VERSION_CODE} exceeds supported range." >&2
  exit 1
fi

RELEASE_TAG="v${RELEASE_VERSION}"
if git rev-parse -q --verify "refs/tags/${RELEASE_TAG}" >/dev/null 2>&1; then
  echo "Resolved stable tag already exists locally: ${RELEASE_TAG}" >&2
  exit 1
fi
if git remote get-url origin >/dev/null 2>&1; then
  if git ls-remote --exit-code --tags origin "refs/tags/${RELEASE_TAG}" >/dev/null 2>&1; then
    echo "Resolved stable tag already exists on origin: ${RELEASE_TAG}" >&2
    exit 1
  fi
fi

if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  {
    echo "release_version=${RELEASE_VERSION}"
    echo "release_tag=${RELEASE_TAG}"
    echo "android_version_code=${ANDROID_VERSION_CODE}"
  } >> "${GITHUB_OUTPUT}"
fi

echo "release_version=${RELEASE_VERSION}"
echo "release_tag=${RELEASE_TAG}"
echo "android_version_code=${ANDROID_VERSION_CODE}"
