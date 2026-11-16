#!/usr/bin/env bash
# Sets up a local MariaDB test server inside WSL/Linux for the smoke tool.
#
# What it creates:
#   - MariaDB from the distro's packages, listening on 127.0.0.1:3307 only
#   - TLS with a private CA (RSA, as real MySQL/MariaDB deployments use)
#   - a Linux user and a matching PAM-authenticated database account that
#     logs in through the mysql_clear_password client plugin, the same
#     client-side path MySQL Enterprise uses for LDAP and PAM accounts
#   - accounts for mysql_native_password and ed25519
#   - a small `sqltool_test` schema (table, view, procedure) the accounts own
#
# Run as root from the repository root:  bash tests/servers/mariadb-pam.sh
# Generated passwords and the CA certificate land in tests/servers/.local/,
# which is ignored by git. Test use only; do not expose this server.
set -euo pipefail

OUT="tests/servers/.local"
CERTS="/etc/mysql/sqltool-test"
PORT=3307
PAM_USER="sqltool_pam"

export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq mariadb-server openssl >/dev/null

mkdir -p "$OUT" "$CERTS"

# --- certificates -----------------------------------------------------------
if [ ! -f "$CERTS/ca.pem" ]; then
  openssl req -x509 -newkey rsa:2048 -nodes -days 365 \
    -subj "/CN=sqltool test CA" \
    -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign" \
    -keyout "$CERTS/ca-key.pem" -out "$CERTS/ca.pem" 2>/dev/null
  openssl req -newkey rsa:2048 -nodes -subj "/CN=sqltool test server" \
    -keyout "$CERTS/server-key.pem" -out "$CERTS/server.csr" 2>/dev/null
  printf 'subjectAltName=DNS:localhost,IP:127.0.0.1\nextendedKeyUsage=serverAuth\n' > "$CERTS/san.ext"
  openssl x509 -req -in "$CERTS/server.csr" -CA "$CERTS/ca.pem" -CAkey "$CERTS/ca-key.pem" \
    -CAcreateserial -days 365 -extfile "$CERTS/san.ext" -out "$CERTS/server-cert.pem" 2>/dev/null
  chown -R mysql:mysql "$CERTS"
  chmod 600 "$CERTS"/*-key.pem
fi
cp "$CERTS/ca.pem" "$OUT/mariadb-ca.pem"

# --- PAM account ------------------------------------------------------------
if [ ! -f "$OUT/mariadb.env" ]; then
  PAM_PASSWORD="$(openssl rand -hex 16)"
  SQL_PASSWORD="$(openssl rand -hex 16)"
  printf 'PAM_PASSWORD=%s\nSQL_PASSWORD=%s\n' "$PAM_PASSWORD" "$SQL_PASSWORD" > "$OUT/mariadb.env"
fi
# shellcheck disable=SC1090
. "$OUT/mariadb.env"

id "$PAM_USER" >/dev/null 2>&1 || useradd --no-create-home --shell /usr/sbin/nologin "$PAM_USER"
echo "$PAM_USER:$PAM_PASSWORD" | chpasswd

cat > /etc/pam.d/sqltool-test <<'PAM'
auth    required pam_unix.so
account required pam_unix.so
PAM

# --- server configuration ---------------------------------------------------
cat > /etc/mysql/mariadb.conf.d/99-sqltool-test.cnf <<CNF
[mariadb]
bind-address = 127.0.0.1
port = $PORT
plugin_load_add = auth_pam
plugin_load_add = auth_ed25519
pam_use_cleartext_plugin = ON
ssl_ca = $CERTS/ca.pem
ssl_cert = $CERTS/server-cert.pem
ssl_key = $CERTS/server-key.pem
CNF

systemctl restart mariadb

mariadb <<SQL
CREATE OR REPLACE USER '$PAM_USER'@'%' IDENTIFIED VIA pam USING 'sqltool-test';
CREATE OR REPLACE USER 'sqltool_native'@'%' IDENTIFIED VIA mysql_native_password USING PASSWORD('$SQL_PASSWORD');
CREATE OR REPLACE USER 'sqltool_ed'@'%' IDENTIFIED VIA ed25519 USING PASSWORD('$SQL_PASSWORD');
GRANT SELECT ON *.* TO '$PAM_USER'@'%', 'sqltool_native'@'%', 'sqltool_ed'@'%';
GRANT ALL ON sqltool_test.* TO '$PAM_USER'@'%', 'sqltool_native'@'%', 'sqltool_ed'@'%';
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
CREATE OR REPLACE VIEW sqltool_test.people_names AS SELECT id, name FROM sqltool_test.people;
DELIMITER //
CREATE OR REPLACE PROCEDURE sqltool_test.two_sets()
BEGIN
  SELECT 1 AS a;
  SELECT 2 AS b, 3 AS c;
END//
DELIMITER ;
SQL

echo "--- server ---"
mariadb -N -e "SELECT VERSION(); SELECT CONCAT('pam cleartext: ', @@pam_use_cleartext_plugin); SELECT CONCAT('tls: ', @@have_ssl, ' ', @@tls_version); SELECT CONCAT(user, ' -> ', plugin) FROM mysql.user WHERE user LIKE 'sqltool%' ORDER BY user;"
echo "listening on 127.0.0.1:$PORT"
