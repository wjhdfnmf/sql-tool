#!/usr/bin/env bash
# Sets up a local MySQL 8.4 test server inside WSL/Linux for the smoke tool.
#
# Ubuntu's mysql-server and mariadb-server packages cannot both be installed,
# so this unpacks the mysql-server-core package into its own folder and runs
# mysqld from there, next to the MariaDB test server, on 127.0.0.1:3308.
#
# Usage, as root from the repository root:
#   bash tests/servers/mysql84.sh auto   # MySQL's auto-generated certificates
#   bash tests/servers/mysql84.sh san    # certificates with a host name (run mariadb-pam.sh first)
#
# Accounts: caching_sha2_password, mysql_native_password, sha256_password,
# plus a small `sqltool_test` schema (table and view) the accounts own.
# Passwords and CA certificates land in tests/servers/.local/ (git-ignored).
set -euo pipefail

MODE="${1:-auto}"
BASE="/opt/sqltool-test/mysql84"
DATA="$BASE/data"
OUT="tests/servers/.local"
SAN_CERTS="/etc/mysql/sqltool-test"
PORT=3308
UNIT="sqltool-mysql84"

export DEBIAN_FRONTEND=noninteractive
mkdir -p "$BASE" "$OUT"

if [ ! -x "$BASE/usr/sbin/mysqld" ]; then
  apt-get update -qq
  # Shared libraries mysqld needs; none of these conflict with MariaDB.
  deps="$(apt-cache depends mysql-server-core | awk '$1 == "Depends:" {print $2}' | grep -Ev '^(<|mysql-(server|client))' | sort -u)"
  # shellcheck disable=SC2086
  apt-get install -y -qq $deps >/dev/null
  tmp="$(mktemp -d)"
  chmod 755 "$tmp"
  (cd "$tmp" && apt-get download -qq mysql-server-core)
  dpkg -x "$tmp"/mysql-server-core_*.deb "$BASE"
  rm -rf "$tmp"
fi

if [ ! -f "$OUT/mysql84.env" ]; then
  printf 'SQL_PASSWORD=%s\n' "$(openssl rand -hex 16)" > "$OUT/mysql84.env"
fi
# shellcheck disable=SC1090
. "$OUT/mysql84.env"

systemctl stop "$UNIT" 2>/dev/null || true
systemctl reset-failed "$UNIT" 2>/dev/null || true

mkdir -p "$BASE/run"
chown mysql:mysql "$BASE/run"

if [ ! -d "$DATA/mysql" ]; then
  # Not initialised yet (or a previous attempt failed part way): start clean.
  rm -rf "$DATA"
  mkdir -p "$DATA"
  chown mysql:mysql "$DATA"
  "$BASE/usr/sbin/mysqld" --no-defaults --initialize-insecure --user=mysql \
    --basedir="$BASE/usr" --datadir="$DATA" --log-error="$BASE/run/init.log"
fi

cat > "$BASE/init.sql" <<SQL
CREATE USER IF NOT EXISTS 'sqltool_sha2'@'%' IDENTIFIED WITH caching_sha2_password BY '$SQL_PASSWORD';
CREATE USER IF NOT EXISTS 'sqltool_native'@'%' IDENTIFIED WITH mysql_native_password BY '$SQL_PASSWORD';
CREATE USER IF NOT EXISTS 'sqltool_sha256'@'%' IDENTIFIED WITH sha256_password BY '$SQL_PASSWORD';
GRANT SELECT ON *.* TO 'sqltool_sha2'@'%', 'sqltool_native'@'%', 'sqltool_sha256'@'%';
GRANT ALL ON sqltool_test.* TO 'sqltool_sha2'@'%', 'sqltool_native'@'%', 'sqltool_sha256'@'%';
CREATE DATABASE IF NOT EXISTS sqltool_test;
CREATE TABLE IF NOT EXISTS sqltool_test.people (
  id INT AUTO_INCREMENT PRIMARY KEY,
  name VARCHAR(100) NOT NULL,
  email VARCHAR(200) NULL,
  born DATE NULL,
  notes TEXT NULL,
  avatar BLOB NULL,
  profile JSON NULL,
  created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  KEY idx_name (name)
);
INSERT INTO sqltool_test.people (id, name, email, born, notes, avatar, profile) VALUES
  (1, 'Ada Lovelace', 'ada@example.test', '1815-12-10', 'First programmer', x'89504E470D0A1A0A', '{"languages": ["notes"], "active": true}'),
  (2, 'Grace Hopper', NULL, '1906-12-09', NULL, NULL, '{"languages": ["cobol"], "active": true}'),
  (3, 'Zoë Müller', 'zoe@example.test', NULL, 'Non-ASCII name: ß, 漢字, 😀', NULL, NULL)
  ON DUPLICATE KEY UPDATE name = VALUES(name);
CREATE OR REPLACE SQL SECURITY INVOKER VIEW sqltool_test.people_names AS SELECT id, name FROM sqltool_test.people;
SQL

cat > "$BASE/my.cnf" <<CNF
[mysqld]
user = mysql
basedir = $BASE/usr
datadir = $DATA
plugin-dir = $BASE/usr/lib/mysql/plugin
port = $PORT
bind-address = 127.0.0.1
mysqlx = OFF
secure-file-priv = NULL
socket = $BASE/run/mysqld.sock
pid-file = $BASE/run/mysqld.pid
log-error = $BASE/run/error.log
mysql-native-password = ON
init-file = $BASE/init.sql
CNF
if [ "$MODE" = "san" ]; then
  cat >> "$BASE/my.cnf" <<CNF
ssl-ca = $SAN_CERTS/ca.pem
ssl-cert = $SAN_CERTS/server-cert.pem
ssl-key = $SAN_CERTS/server-key.pem
CNF
  cp "$SAN_CERTS/ca.pem" "$OUT/mysql84-ca.pem"
else
  cp "$DATA/ca.pem" "$OUT/mysql84-ca.pem"
fi

mkdir -p "$BASE/run"
chown mysql:mysql "$BASE/run" "$BASE/init.sql" "$BASE/my.cnf"
chmod 600 "$BASE/init.sql"
: > "$BASE/run/error.log"
chown mysql:mysql "$BASE/run/error.log"

systemd-run --quiet --unit="$UNIT" --collect "$BASE/usr/sbin/mysqld" --defaults-file="$BASE/my.cnf"

for _ in $(seq 1 60); do
  if ss -ltn | grep -q ":$PORT\b"; then
    echo "MySQL $("$BASE/usr/sbin/mysqld" --no-defaults --version | awk '{print $3}') listening on 127.0.0.1:$PORT, certificates: $MODE"
    openssl x509 -in "$( [ "$MODE" = san ] && echo "$SAN_CERTS/server-cert.pem" || echo "$DATA/server-cert.pem" )" \
      -noout -subject -ext subjectAltName 2>/dev/null | sed 's/^/  /' || true
    exit 0
  fi
  sleep 1
done
echo "mysqld did not start; last log lines:" >&2
tail -20 "$BASE/run/error.log" >&2
exit 1
