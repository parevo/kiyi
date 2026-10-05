#!/bin/bash
# The same seed in a separate database for the live tests, so editing `shop` in the app never breaks them.
set -e
createdb -U "$POSTGRES_USER" kiyi_test
psql -v ON_ERROR_STOP=1 -U "$POSTGRES_USER" -d kiyi_test -f /docker-entrypoint-initdb.d/01-seed.sql
