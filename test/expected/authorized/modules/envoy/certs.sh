#!/usr/bin/env bash
# Mint a throwaway CA and a leaf certificate for the hostnames the gateway emulates, plus a
# JVM truststore (JDK defaults + the CA). Idempotent: the CA persists in the named volume, so
# clients that already trust it keep working. The leaf is re-minted only when the host list
# (CERT_HOSTS) changes.
set -euo pipefail
cd "$CERT_DIR"

if [[ ! -f ca.pem || ! -f ca-key.pem || ! -f truststore.p12 ]]; then
  openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
    -subj "/CN=trestle local CA" -keyout ca-key.pem -out ca.pem
  rm -f truststore.p12 leaf.pem leaf.san
  cp "$JAVA_HOME/lib/security/cacerts" truststore.p12
  keytool -importcert -noprompt -alias trestle-local-ca -file ca.pem \
    -keystore truststore.p12 -storepass changeit
fi

# shellcheck disable=SC2086 # split the space-separated host list
san=$(printf 'DNS:%s\n' $CERT_HOSTS | paste -sd, -)

if [[ -f leaf.pem && -f leaf.san && "$(cat leaf.san)" == "$san" ]]; then
  echo "certs up to date"
  exit 0
fi

cat > leaf.ext <<EXT
basicConstraints=CA:FALSE
keyUsage=digitalSignature,keyEncipherment
extendedKeyUsage=serverAuth
subjectAltName=$san
EXT

openssl req -newkey rsa:2048 -nodes \
  -subj "/CN=trestle gateway" -keyout leaf-key.pem -out leaf.csr
openssl x509 -req -in leaf.csr -CA ca.pem -CAkey ca-key.pem -CAcreateserial \
  -days 825 -extfile leaf.ext -out leaf.pem
printf '%s' "$san" > leaf.san

# The gateway and its consumers run as non-root users; they only need to read these.
chmod 0644 ./*.pem truststore.p12
rm -f leaf.csr leaf.ext ca.srl