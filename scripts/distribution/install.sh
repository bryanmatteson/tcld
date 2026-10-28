#!/bin/sh
# Download the Python installer; it verifies and installs the complete archive.
set -eu
base=${TCLD_RELEASE_BASE_URL:-https://github.com/bryanmatteson/tcld/releases/latest/download}
previous=
for argument do
  if [ "$previous" = --base-url ]; then base=$argument; fi
  previous=$argument
done
command -v python3 >/dev/null 2>&1 || { echo 'Python 3.9+ is required' >&2; exit 1; }
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
python3 - "$base" "$temporary/install.py" <<'PY'
import pathlib, sys, urllib.parse, urllib.request
url = sys.argv[1].rstrip('/') + '/install.py'
p = urllib.parse.urlparse(url)
if p.scheme != 'https' and not (p.scheme == 'http' and p.hostname in ('localhost', '127.0.0.1', '::1')):
    sys.exit('Installer URL requires HTTPS')
with urllib.request.urlopen(url, timeout=120) as r:
    final = urllib.parse.urlparse(r.url)
    if final.scheme != 'https' and final.hostname not in ('localhost', '127.0.0.1', '::1'):
        sys.exit('Installer redirected to an insecure URL')
    pathlib.Path(sys.argv[2]).write_bytes(r.read())
PY
python3 "$temporary/install.py" "$@"
