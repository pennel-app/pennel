#!/bin/sh
set -eu
root="${CI_PRIMARY_REPOSITORY_PATH:-$(CDPATH= cd -- "$(dirname -- "$0")/../../../.." && pwd)}"
exec /bin/bash "$root/scripts/xcode-cloud-post-clone.sh"
