#!/bin/bash
set -euo pipefail

# Migo Release Publisher
# Usage: ./scripts/publish-release.sh <version> <artifacts-dir>
# Example: ./scripts/publish-release.sh v0.9.0 /tmp/migo-artifacts

VERSION="${1:-}"
ARTIFACTS_DIR="${2:-}"

if [[ -z "$VERSION" ]] || [[ -z "$ARTIFACTS_DIR" ]]; then
  echo "Usage: $0 <version> <artifacts-dir>"
  echo ""
  echo "Example:"
  echo "  $0 v0.9.0 /tmp/migo-artifacts"
  echo ""
  echo "Expected artifacts directory structure:"
  echo "  /tmp/migo-artifacts/"
  echo "  ├─ migo-android-java.aar"
  echo "  ├─ migo-android-capi-arm64-v8a.tar.gz"
  echo "  ├─ migo-android-capi-x86_64.tar.gz"
  echo "  ├─ migo-linux-x86_64.tar.gz"
  echo "  └─ migo-windows-x86_64.tar.gz"
  exit 1
fi

if [[ ! -d "$ARTIFACTS_DIR" ]]; then
  echo "❌ Artifacts directory not found: $ARTIFACTS_DIR"
  exit 1
fi

echo "【Migo Release Publisher】"
echo "Version: $VERSION"
echo "Artifacts: $ARTIFACTS_DIR"
echo ""

# Verify we're in a git repo
if [[ ! -d .git ]]; then
  echo "❌ Not in a git repository"
  exit 1
fi

# Step 1: Generate attestation files
#
# Always (re)generate, even if an attestation file already exists under this
# name: a leftover attestation from an earlier build (e.g. one produced by
# package-sdk.sh under a different source filename, then dragged along when
# the artifact was renamed to match this script's naming) can be present but
# stale, with a package_file/size/sha256 that no longer matches the actual
# bytes being uploaded. Consumers verify the attestation against the asset
# they downloaded, so a stale attestation here is a silent release-breaking
# bug rather than a build-time failure -- it only surfaces downstream when
# resolve-migo-artifact.sh rejects the mismatch. Regenerating unconditionally
# from the file about to be uploaded is the only way to guarantee they agree.
echo "📝 Generating attestation files..."
cd "$ARTIFACTS_DIR"
for file in migo-*.aar migo-*.tar.gz; do
  if [[ ! -f "$file" ]]; then
    continue
  fi

  size=$(stat -c%s "$file")
  sha256=$(sha256sum "$file" | awk '{print $1}')

  cat > "${file}.attestation.json" <<EOF
{
  "schema": "migo-release-attestation/v1",
  "package_file": "$file",
  "package_size_bytes": $size,
  "package_sha256": "$sha256"
}
EOF
  echo "  ✓ ${file}.attestation.json"
done

# Step 2: Verify expected artifacts exist
echo ""
echo "📦 Verifying artifacts..."
required_files=(
  "migo-android-java.aar"
  "migo-android-capi-arm64-v8a.tar.gz"
  "migo-android-capi-x86_64.tar.gz"
  "migo-linux-x86_64.tar.gz"
  "migo-windows-x86_64.tar.gz"
)

missing_files=()
for file in "${required_files[@]}"; do
  if [[ ! -f "$file" ]]; then
    missing_files+=("$file")
  else
    echo "  ✓ $file"
  fi
done

if [[ ${#missing_files[@]} -gt 0 ]]; then
  echo ""
  echo "❌ Missing artifacts:"
  for file in "${missing_files[@]}"; do
    echo "  - $file"
  done
  exit 1
fi

# Step 3: Create git tag (from repo root)
cd - > /dev/null
echo ""
echo "🏷️  Creating git tag..."
if git rev-parse "$VERSION" > /dev/null 2>&1; then
  echo "❌ Tag already exists: $VERSION"
  exit 1
fi

git tag "$VERSION"
git push origin "$VERSION"
echo "  ✓ Tag pushed: $VERSION"

# Step 4: Create release notes
echo ""
echo "📄 Creating release notes..."
RELEASE_NOTES=$(mktemp)
cat > "$RELEASE_NOTES" <<'EOF'
# Migo v0.9.0

Production-ready runtime SDKs for Android, Linux, and Windows.

## Android

### Java SDK
- `migo-android-java.aar` — Runtime library for Android apps (arm64-v8a and x86_64)

### C API
- `migo-android-capi-arm64-v8a.tar.gz` — C headers and static library for arm64-v8a
- `migo-android-capi-x86_64.tar.gz` — C headers and static library for x86_64

## Linux
- `migo-linux-x86_64.tar.gz` — SDK for x86_64 (static and shared libraries, pkg-config, CMake)

## Windows
- `migo-windows-x86_64.tar.gz` — SDK for x86_64 (migo.dll, import library, CMake)

## Verification

Each artifact includes an `.attestation.json` file with SHA256. Verify before use:
```bash
sha256sum -c migo-android-java.aar.attestation.json
```

## Integration

Use [migo-examples](https://github.com/minigame-labs/migo-examples) for runnable samples. See [BUILD.md](../../BUILD.md) for building from source.
EOF

# Step 5: Create GitHub release
echo "🚀 Creating GitHub release..."
gh release create "$VERSION" \
  --title "Migo $VERSION" \
  --notes-file "$RELEASE_NOTES"
echo "  ✓ Release created: $VERSION"

# Step 6: Upload artifacts
echo ""
echo "📤 Uploading artifacts..."
cd "$ARTIFACTS_DIR"
for file in migo-*.aar migo-*.tar.gz migo-*.attestation.json; do
  if [[ ! -f "$file" ]]; then
    continue
  fi
  gh release upload "$VERSION" "$file" --clobber
  echo "  ✓ $file"
done

rm "$RELEASE_NOTES"

# Step 7: Verify
echo ""
echo "✅ Verifying release..."
count=$(gh release view "$VERSION" --json assets --jq '.assets | length')
echo "  Artifacts uploaded: $count"

if [[ $count -eq 10 ]]; then
  echo ""
  echo "✅ Release complete: $VERSION"
  echo "📍 https://github.com/minigame-labs/migo/releases/tag/$VERSION"
else
  echo ""
  echo "⚠️  Expected 10 artifacts, got $count"
  exit 1
fi
