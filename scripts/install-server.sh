#!/usr/bin/env bash
# Install (or update) the management server (server/) as the systemd
# service oswst-server (server/oswst-server.service): build it, copy the
# binary to /usr/local/bin/oswst-server, and (re)start it.
#
# The first run also makes the oswst system user and lets it read DIR: an
# ACL on the directory (and a default one, so a new bundle.tar.gz or an
# edited radios.toml stays readable) plus one on server.key, which stays
# mode 0600 for its owner. DIR must already hold server.key (keygen, see
# the README).
#
# Run it again after changing server/ to install the new build.
#
# Usage: scripts/install-server.sh   (asks for sudo)

set -e

DIR=/srv/oswst
cd "$(dirname "$0")/.."

(cd server && cargo build --release)

if ! getent passwd oswst > /dev/null; then
    sudo useradd --system --no-create-home --shell /usr/sbin/nologin oswst
fi
sudo setfacl -m u:oswst:rx -m d:u:oswst:r "$DIR"
sudo setfacl -m u:oswst:r "$DIR/server.key"

# Under the target triple: server/.cargo/config.toml names one explicitly
sudo install -m 0755 server/target/x86_64-unknown-linux-gnu/release/server /usr/local/bin/oswst-server
sudo install -m 0644 server/oswst-server.service /etc/systemd/system/oswst-server.service
sudo systemctl daemon-reload
sudo systemctl enable oswst-server
sudo systemctl restart oswst-server

sleep 1
systemctl --no-pager status oswst-server
