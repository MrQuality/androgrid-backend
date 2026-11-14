#!/usr/bin/env bash
# Mutual TLS (mTLS 1.3) Cluster PKI Generator
# Traces to: AG-SPEC-2026-V2 Section 2.3, [INV-002], [NFR-SEC-001]
#
# Generates ECDSA prime256v1 (P-256) X.509 v3 certificates for the cluster:
# 1. Internal Root CA (ca.crt, ca.key)
# 2. Host Orchestrator Server Certificate with SANs (server.crt, server.key)
# 3. Android Worker Client Certificate and PKCS12 bundle (client.crt, client.key, client.p12)

set -euo pipefail

OUTPUT_DIR="${1:-./pki}"
DAYS_VALID=1825 # 5 years for internal root CA
CLIENT_DAYS_VALID=365 # 1 year for worker nodes
NODE_ID="${2:-node_worker_01}"
P12_PASSWORD="${3:-androgrid_secure_pki}"
ADDITIONAL_SAN="${4:-}"

echo "=========================================================="
echo "AndroGrid Cluster mTLS 1.3 PKI Generation"
echo "Target Directory: ${OUTPUT_DIR}"
echo "Mobile Node ID:   ${NODE_ID}"
echo "=========================================================="

mkdir -p "${OUTPUT_DIR}"

# 1. Generate Internal Root CA
if [ ! -f "${OUTPUT_DIR}/ca.key" ]; then
    echo "[+] Generating Internal Root CA key (prime256v1)..."
    openssl ecparam -name prime256v1 -genkey -noout -out "${OUTPUT_DIR}/ca.key"
    chmod 600 "${OUTPUT_DIR}/ca.key"

    echo "[+] Generating Self-Signed Root CA Certificate..."
    openssl req -new -x509 -sha256 -key "${OUTPUT_DIR}/ca.key" \
        -out "${OUTPUT_DIR}/ca.crt" -days "${DAYS_VALID}" \
        -subj "/CN=AndroGrid Internal Root CA/O=AndroGrid Cluster/OU=Infrastructure"
else
    echo "[*] Existing Root CA found; reusing ${OUTPUT_DIR}/ca.crt"
fi

# 2. Generate Orchestrator Server Certificate
echo "[+] Generating Orchestrator Server key (prime256v1)..."
openssl ecparam -name prime256v1 -genkey -noout -out "${OUTPUT_DIR}/server.key"
chmod 600 "${OUTPUT_DIR}/server.key"

echo "[+] Creating Orchestrator Server CSR..."
openssl req -new -key "${OUTPUT_DIR}/server.key" \
    -out "${OUTPUT_DIR}/server.csr" \
    -subj "/CN=localhost/O=AndroGrid Cluster/OU=Control Plane"

echo "[+] Signing Orchestrator Server Certificate with Subject Alternative Names (SAN)..."
cat > "${OUTPUT_DIR}/server_ext.cnf" <<EOF
basicConstraints = CA:FALSE
keyUsage = digitalSignature, keyEncipherment
extendedKeyUsage = serverAuth
subjectAltName = @alt_names

[alt_names]
DNS.1 = localhost
DNS.2 = grpc.cluster.domain.com
DNS.3 = host.docker.internal
IP.1 = 127.0.0.1
IP.2 = 0.0.0.0
IP.3 = 10.0.2.2
EOF

if [ -n "${ADDITIONAL_SAN}" ]; then
    if echo "${ADDITIONAL_SAN}" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$'; then
        echo "IP.4 = ${ADDITIONAL_SAN}" >> "${OUTPUT_DIR}/server_ext.cnf"
    else
        echo "DNS.4 = ${ADDITIONAL_SAN}" >> "${OUTPUT_DIR}/server_ext.cnf"
    fi
fi

openssl x509 -req -in "${OUTPUT_DIR}/server.csr" \
    -CA "${OUTPUT_DIR}/ca.crt" -CAkey "${OUTPUT_DIR}/ca.key" -CAcreateserial \
    -out "${OUTPUT_DIR}/server.crt" -days "${CLIENT_DAYS_VALID}" -sha256 \
    -extfile "${OUTPUT_DIR}/server_ext.cnf"

# Bundle Root CA with leaf server certificate to form fullchain for mobile trust pinning
cat "${OUTPUT_DIR}/ca.crt" >> "${OUTPUT_DIR}/server.crt"

rm -f "${OUTPUT_DIR}/server.csr" "${OUTPUT_DIR}/server_ext.cnf"

# 3. Generate Android Worker Client Certificate
echo "[+] Generating Android Worker Client key (prime256v1)..."
openssl ecparam -name prime256v1 -genkey -noout -out "${OUTPUT_DIR}/client.key"
chmod 600 "${OUTPUT_DIR}/client.key"

echo "[+] Creating Android Worker CSR for ${NODE_ID}..."
openssl req -new -key "${OUTPUT_DIR}/client.key" \
    -out "${OUTPUT_DIR}/client.csr" \
    -subj "/CN=${NODE_ID}/O=AndroGrid Worker/OU=Edge Compute"

cat > "${OUTPUT_DIR}/client_ext.cnf" <<EOF
basicConstraints = CA:FALSE
keyUsage = digitalSignature, keyEncipherment
extendedKeyUsage = clientAuth
EOF

openssl x509 -req -in "${OUTPUT_DIR}/client.csr" \
    -CA "${OUTPUT_DIR}/ca.crt" -CAkey "${OUTPUT_DIR}/ca.key" -CAcreateserial \
    -out "${OUTPUT_DIR}/client.crt" -days "${CLIENT_DAYS_VALID}" -sha256 \
    -extfile "${OUTPUT_DIR}/client_ext.cnf"

rm -f "${OUTPUT_DIR}/client.csr" "${OUTPUT_DIR}/client_ext.cnf"

# 4. Package Android KeyStore PKCS12 Bundle (.p12)
echo "[+] Packaging Android KeyStore PKCS12 (.p12) bundle..."
openssl pkcs12 -export \
    -in "${OUTPUT_DIR}/client.crt" \
    -inkey "${OUTPUT_DIR}/client.key" \
    -certfile "${OUTPUT_DIR}/ca.crt" \
    -out "${OUTPUT_DIR}/client.p12" \
    -name "${NODE_ID}" \
    -password "pass:${P12_PASSWORD}"

# 5. Generate Rogue Untrusted Client Certificate for [INV-002] Verification
echo "[+] Generating Rogue CA and Untrusted Client Certificate for [INV-002] mTLS verification..."
openssl ecparam -name prime256v1 -genkey -noout -out "${OUTPUT_DIR}/untrusted_ca.key"
chmod 600 "${OUTPUT_DIR}/untrusted_ca.key"
openssl req -new -x509 -sha256 -key "${OUTPUT_DIR}/untrusted_ca.key" \
    -out "${OUTPUT_DIR}/untrusted_ca.crt" -days 30 \
    -subj "/CN=Untrusted Rogue CA/O=Rogue Cluster/OU=Testing"

openssl ecparam -name prime256v1 -genkey -noout -out "${OUTPUT_DIR}/untrusted_client.key"
chmod 600 "${OUTPUT_DIR}/untrusted_client.key"
openssl req -new -key "${OUTPUT_DIR}/untrusted_client.key" \
    -out "${OUTPUT_DIR}/untrusted_client.csr" \
    -subj "/CN=untrusted_worker/O=Rogue Worker/OU=Edge Compute"
openssl x509 -req -in "${OUTPUT_DIR}/untrusted_client.csr" \
    -CA "${OUTPUT_DIR}/untrusted_ca.crt" -CAkey "${OUTPUT_DIR}/untrusted_ca.key" -CAcreateserial \
    -out "${OUTPUT_DIR}/untrusted_client.crt" -days 30 -sha256
rm -f "${OUTPUT_DIR}/untrusted_client.csr"

echo "=========================================================="
echo "PKI Generation Complete!"
echo "Files generated in: ${OUTPUT_DIR}"
echo "  - ca.crt       : Root CA Public Certificate"
echo "  - server.crt   : Orchestrator TLS 1.3 Fullchain Certificate (Server + Root CA)"
echo "  - server.key   : Orchestrator Private Key"
echo "  - client.crt   : Worker TLS 1.3 Certificate"
echo "  - client.key   : Worker Private Key"
echo "  - client.p12   : PKCS12 bundle for Android KeyStore import"
echo "=========================================================="
