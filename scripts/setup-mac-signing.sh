#!/bin/bash
# One-time setup: a self-signed code-signing certificate in the login keychain,
# so every build of the app is signed with the same identity.
#
# Why: ad hoc signing ("-") gives each build a new code hash, and macOS pins the
# Screen Recording grant to that hash. After a rebuild the toggle in System
# Settings still looks on, but capture silently fails. A certificate-based
# signature keeps the grant valid across rebuilds.
#
# Idempotent: does nothing when the identity already exists. No Apple developer
# account needed. Override the name with MEETILY_SIGNING_IDENTITY and the
# keychain with MEETILY_KEYCHAIN (used by tests).
set -euo pipefail

IDENTITY="${MEETILY_SIGNING_IDENTITY:-meetily-fork local}"
KEYCHAIN="${MEETILY_KEYCHAIN:-$HOME/Library/Keychains/login.keychain-db}"

# A usable identity's line ends right after the quoted name. An untrusted one carries a
# trailing error, e.g. `"name" (CSSMERR_TP_NOT_TRUSTED)`, and codesign cannot use it.
valid_identity() {
  security find-identity -v -p codesigning "$KEYCHAIN" | grep -Eq "\"$IDENTITY\"[[:space:]]*$"
}

trust_notice() {
  echo "A macOS password window will appear to trust the certificate. It may open behind other windows: check the Dock and press Cmd-Tab if nothing shows."
}

if valid_identity; then
  echo "Signing identity \"$IDENTITY\" already exists in $KEYCHAIN. Nothing to do."
  exit 0
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# The certificate is there but not trusted yet (an interrupted earlier run): trust it, do not make a second.
if security find-certificate -c "$IDENTITY" "$KEYCHAIN" >/dev/null 2>&1; then
  security find-certificate -c "$IDENTITY" -p "$KEYCHAIN" > "$work/existing.pem"
  trust_notice
  security add-trusted-cert -p codeSign -k "$KEYCHAIN" "$work/existing.pem"
  valid_identity || { echo "The certificate exists but codesign cannot use it." >&2; exit 1; }
  echo "Trusted the existing signing identity \"$IDENTITY\"."
  exit 0
fi

cat > "$work/openssl.cnf" <<CNF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $IDENTITY
[ext]
basicConstraints = critical,CA:false
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
CNF

# 10 years; the password only protects the temporary .p12 used for the import.
pass="meetily-$$"
/usr/bin/openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
  -keyout "$work/key.pem" -out "$work/cert.pem" -config "$work/openssl.cnf" 2>/dev/null
/usr/bin/openssl pkcs12 -export -inkey "$work/key.pem" -in "$work/cert.pem" \
  -out "$work/id.p12" -passout "pass:$pass" 2>/dev/null

# -T lets codesign use the key without a prompt on every build.
security import "$work/id.p12" -k "$KEYCHAIN" -P "$pass" -T /usr/bin/codesign >/dev/null

# codesign only accepts a self-signed certificate that is trusted for code
# signing. This sets that trust for the current user (macOS may ask for your
# login password once).
trust_notice
security add-trusted-cert -p codeSign -k "$KEYCHAIN" "$work/cert.pem"

valid_identity || { echo "Imported the certificate but codesign cannot use it yet." >&2; exit 1; }

echo "Created signing identity \"$IDENTITY\" in $KEYCHAIN."
echo "Build with: APPLE_SIGNING_IDENTITY=\"$IDENTITY\" pnpm run tauri:build (scripts/install-mac.sh --build does this)."
echo "macOS may ask once for your login password the first time codesign uses the key: choose Always Allow."
