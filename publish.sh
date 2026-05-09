#!/bin/bash
set -euxo pipefail

NEW_VERSION=$1

echo "new_release=true" >> "$GITHUB_OUTPUT"
echo "version=${NEW_VERSION}" >> "$GITHUB_OUTPUT"