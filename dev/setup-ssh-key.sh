#!/bin/sh
# Creates the key the bastion accepts (dev/ssh/id_ed25519). Local only; never committed.
set -e
cd "$(dirname "$0")"
mkdir -p ssh
[ -f ssh/id_ed25519 ] || ssh-keygen -q -t ed25519 -N "" -C kiyi-dev -f ssh/id_ed25519
