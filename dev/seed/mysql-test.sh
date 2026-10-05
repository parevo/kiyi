#!/bin/bash
# The same seed in a separate database for the live tests, so editing `shop` in the app never breaks them.
set -e
mysql -uroot -p"$MYSQL_ROOT_PASSWORD" -e "CREATE DATABASE kiyi_test DEFAULT CHARSET utf8mb4; GRANT ALL ON kiyi_test.* TO 'kiyi'@'%';"
mysql -uroot -p"$MYSQL_ROOT_PASSWORD" --default-character-set=utf8mb4 kiyi_test < /docker-entrypoint-initdb.d/01-seed.sql
