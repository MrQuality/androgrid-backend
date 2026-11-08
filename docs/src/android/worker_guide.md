# Android Worker Node Onboarding & Contributor Guide

Welcome to the **AndroGrid Mobile Worker** onboarding manual. This guide walks you through provisioning an Android device (physical smartphone, tablet, or emulator) to participate as a distributed edge-compute worker node in the AndroGrid cluster.

Traceability: `[REQ-OP-001]`, `[REQ-OP-002]`, `[REQ-OP-003]`, `[REQ-VER-001]`, `[REQ-VER-002]`, `[INV-001]`, `[INV-002]`, `[INV-003]`, `[NFR-POW-001]`.

<div class="callout callout-info">
  <strong>Interactive Kotlin API Reference:</strong> For code symbols, classes, and KDoc method signatures, visit the dedicated <a href="https://mrquality.github.io/androgrid-android/dokka/index.html" target="_blank">Android Dokka API Documentation &nearr;</a>.
</div>

---

## Device Prerequisites & Compatibility Matrix

Before enrolling a device, verify that it meets the following hardware and operating system requirements:

| Metric | Minimum Requirement | Recommended Specification | Rationale & Traceability |
| :--- | :--- | :--- | :--- |
| **Android Version** | Android 9 (Pie, API 28) | Android 14+ (API 34) | Min SDK 28 is required for core crypto libraries and desugared Java time APIs. API 34+ enforces typed foreground services (`[REQ-VER-001]`). |
| **RAM** | 2.0 GB Total RAM | 4.0 GB+ LPDDR4X | Ensures sufficient head-room for in-memory WebAssembly bytecode execution without triggering Android low-memory killer (LMK). |
| **Free Storage** | 500 MB | 2.0 GB | Storage for local WASM bytecode cache and execution logs. |
| **Network Interfaces** | Wi-Fi (802.11n/ac) or Cellular 4G/5G | Dual Wi-Fi 6 + 5G NR | Supports dual-route operation: high-bandwidth Wi-Fi sync and low-power cellular spot execution. |
| **Root Status** | Unrooted (`is_rooted = false`) | Rooted or Unrooted | Unrooted devices safely execute WASM payloads within the in-process sandbox (`[INV-006]`). Rooted devices can optionally execute unprivileged shell benchmarks. |

---

## Step 1: Installing the Worker APK via ADB

You can install the pre-compiled debug or release APK directly onto your device using the Android Debug Bridge (ADB).

```bash
# Enable USB Debugging in Developer Options on your device, then connect via USB:
adb devices

# Install or upgrade the worker application:
adb install -r -g ./app/build/outputs/apk/debug/app-debug.apk
```

### Command Argument Breakdown

| Argument / Flag | Purpose & Operational Impact |
| :--- | :--- |
| `adb install` | Initiates the package manager installation stream over ADB transport to `/data/local/tmp`. |
| `-r` (Replace) | Reinstalls an existing application while keeping all user settings, cluster certificates, and cached task history intact. |
| `-g` (Grant Permissions) | Automatically grants all requested runtime permissions declared in `AndroidManifest.xml` (including `POST_NOTIFICATIONS`, `ACCESS_NETWORK_STATE`, and `FOREGROUND_SERVICE_DATA_SYNC`). Prevents manual prompt dialogs during headless onboarding. |

---

## Step 2: Generating and Provisioning mTLS 1.3 Certificates

All control plane communications require Mutual TLS 1.3 (`[INV-002]`, `[NFR-SEC-001]`). The Android client requires a private key and X.509 certificate signed by the cluster Root CA.

### Generating the Client Key and Certificate with OpenSSL

If you are enrolling a new device named `node_pixel_7_01`, run the following commands on your cluster management workstation:

```bash
# 1. Generate an unencrypted ECDSA private key using the NIST P-256 curve:
openssl ecparam -name prime256v1 -genkey -noout -out client_node_pixel_7_01.key

# 2. Create a Certificate Signing Request (CSR) specifying the device identity:
openssl req -new -key client_node_pixel_7_01.key \
  -out client_node_pixel_7_01.csr \
  -subj "/CN=node_pixel_7_01/O=AndroGrid Cluster/OU=Mobile Worker"

# 3. Sign the CSR with the cluster Root CA (valid for 365 days):
openssl x509 -req -in client_node_pixel_7_01.csr \
  -CA deploy/pki/ca.crt -CAkey deploy/pki/ca.key -CAcreateserial \
  -out client_node_pixel_7_01.crt -days 365 -sha256

# 4. Package the certificate, private key, and Root CA into an encrypted PKCS#12 bundle:
openssl pkcs12 -export \
  -inkey client_node_pixel_7_01.key \
  -in client_node_pixel_7_01.crt \
  -certfile deploy/pki/ca.crt \
  -out client.p12 \
  -password pass:androgrid_secure_pki
```

### OpenSSL Argument Reference

| Flag / Option | Cryptographic Purpose & Architectural Significance |
| :--- | :--- |
| `ecparam -name prime256v1` | Selects NIST curve P-256 (FIPS 186-4). Provides 128-bit security with ultra-fast scalar multiplication on mobile ARM processors. |
| `-genkey -noout` | Directly emits the private key in SEC1/PKCS#8 PEM format without printing curve parameter metadata. |
| `-subj "/CN=.../O=..."` | Sets X.509 Distinguished Name fields. The Common Name (CN) is used by the orchestrator for node audit logging. |
| `-export -out client.p12` | Constructs a PKCS#12 (PFX) cryptographic archive containing both the public certificate chain and the private key. |
| `-password pass:androgrid_secure_pki` | Encrypts the PKCS#12 keystore using password-based encryption, protecting the private key in transit. |

### Provisioning `client.p12` to the Android Device

Push the generated `client.p12` and `ca.crt` to the device storage or import via the in-app settings UI:

```bash
# Push PKCS#12 file to device downloads folder:
adb push client.p12 /sdcard/Download/client.p12
```

In the AndroGrid Worker application:
1. Open **Settings** (gear icon in the top right).
2. Tap **Import Client Certificate**.
3. Select `/sdcard/Download/client.p12` and enter password `androgrid_secure_pki`.
4. The key is securely stored in the hardware-backed Android KeyStore.

---

## Step 3: Connecting to the Control Plane Orchestrator

In the application settings, configure the orchestrator target endpoint:

- **Local Machine Testing with Podman (Android Emulator):**
  Use `https://10.0.2.2:50051`. In the Android emulator, `10.0.2.2` automatically routes to host machine `localhost` where Podman publishes port `50051`.
- **Local Machine Testing with Podman (Physical Device over Wi-Fi):**
  Use `https://192.168.1.100:50051` (replace with your workstation local LAN IP address).
- **Remote Cellular WAN Ingress (Cloudflare Tunnel):**
  Use `https://grpc.cluster.domain.com`.

---

## Step 4: OEM Battery Optimization Exemption Guidelines

Android device manufacturers (OEMs) implement aggressive proprietary battery managers that terminate background foreground services unless explicitly whitelisted.

### 1. Samsung (One UI)
- Open **Settings** -> **Apps** -> **AndroGrid Worker**.
- Tap **Battery** -> Select **Unrestricted**.
- Go to **Device Care** -> **Battery** -> **Background usage limits** -> Add AndroGrid Worker to **Never sleeping apps**.

### 2. Xiaomi / Redmi / POCO (MIUI & HyperOS)
- Open **Settings** -> **Apps** -> **Manage Apps** -> **AndroGrid Worker**.
- Enable **Autostart** toggle.
- Under **Battery saver**, select **No restrictions**.
- In the Recents app switcher, long-press AndroGrid Worker and tap the **Lock icon** to prevent task clearance.

### 3. Huawei / Honor (EMUI & MagicOS)
- Open **Settings** -> **Battery** -> **App Launch**.
- Find **AndroGrid Worker**, toggle off **Manage automatically**, and ensure **Auto-launch**, **Secondary launch**, and **Run in background** are all enabled.

### 4. Google Pixel & Motorola (Stock Android)
- Open **Settings** -> **Apps** -> **See all apps** -> **AndroGrid Worker**.
- Tap **App battery usage** -> Select **Unrestricted**.

---

## Step 5: Verifying Worker Enrollment

Once started:
1. Tap **Start Worker** in the main dashboard.
2. A persistent notification appears indicating:
   `AndroGrid Worker: Connected (Wi-Fi LAN) - Ready for spot tasks`
3. On your host machine, run:
   ```bash
   ./target/release/androgrid-cli \
     --endpoint https://127.0.0.1:50051 \
     --ca-cert ./deploy/pki/ca.crt \
     --client-cert ./deploy/pki/client.crt \
     --client-key ./deploy/pki/client.key \
     nodes
   ```
4. Your Android device will appear in the active worker node inventory!
