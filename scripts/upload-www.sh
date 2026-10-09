#!/usr/bin/env bash
# Upload the web app (www/) to a radio's storage over WebDAV, to /fs/www/.
# Then browse to http://<radio>/fs/www/index.html
#
# Usage: scripts/upload-www.sh <radio address or name>   e.g. 192.168.0.219

set -e

RADIO="${1:?usage: $0 <radio address or name>}"
cd "$(dirname "$0")/../www"

# The folder may already be there: MKCOL then answers 405, which is fine
curl -s -o /dev/null -X MKCOL "http://$RADIO/fs/www"
for file in *; do
    curl -sf -T "$file" "http://$RADIO/fs/www/$file" > /dev/null
    echo "uploaded $file"
done
echo "Open http://$RADIO/fs/www/index.html"
