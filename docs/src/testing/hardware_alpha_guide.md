# Hardware Alpha Testing & Verification Guide (v0.0.1-alpha.1)

Welcome to the **AndroGrid Hardware Alpha Testing** guide. This manual provides an end-to-end testing procedure for early hardware testers running on physical Android devices.

Traceability: `[REQ-OP-001]`, `[REQ-OP-002]`, `[REQ-OP-003]`, `[REQ-VER-001]`, `[REQ-VER-002]`, `[INV-001]`, `[INV-002]`, `[INV-003]`, `[NFR-POW-001]`, `[NFR-PERF-001]`.

---

## 1. Overview & Testing Philosophy

The `v0.0.1-alpha.1` release is designed for real-world dogfooding on physical smartphones and tablets across heterogeneous silicon, Android OS versions, and thermal conditions.

As an alpha tester, **you do not need to clone repositories, compile code, or open Pull Requests.** Your role is strictly to:
1. Download pre-compiled release binaries and APKs.
2. Execute the verification steps on your device.
3. Report your execution results and file bug reports directly using **GitHub Issues**.

---

## 2. Download Release Assets (v0.0.1-alpha.1)

All immutable binary packages, mobile application bundles, and pre-compiled WASM workloads are hosted directly as release assets on GitHub:

* **GitHub Release Page:** [AndroGrid v0.0.1-alpha.1 Release Assets](https://github.com/MrQuality/androgrid-backend/releases/tag/v0.0.1-alpha.1)

### Asset Download Directory

| Component | Target Platform | Release Asset Link |
| :--- | :--- | :--- |
| **Android Worker APK** | Android 9+ (API 28 to 35) | [`androgrid-worker-v0.0.1-alpha.1.apk`](https://github.com/MrQuality/androgrid-android/releases/download/v0.0.1-alpha.1/androgrid-worker-v0.0.1-alpha.1.apk) |
| **CLI & Orchestrator** | Linux (x86_64) | [`androgrid-v0.0.1-alpha.1-x86_64-unknown-linux-gnu.tar.gz`](https://github.com/MrQuality/androgrid-backend/releases/download/v0.0.1-alpha.1/androgrid-v0.0.1-alpha.1-x86_64-unknown-linux-gnu.tar.gz) |
| **CLI & Orchestrator** | Linux (ARM64 / aarch64) | [`androgrid-v0.0.1-alpha.1-aarch64-unknown-linux-gnu.tar.gz`](https://github.com/MrQuality/androgrid-backend/releases/download/v0.0.1-alpha.1/androgrid-v0.0.1-alpha.1-aarch64-unknown-linux-gnu.tar.gz) |
| **CLI & Orchestrator** | Windows (x86_64) | [`androgrid-v0.0.1-alpha.1-x86_64-pc-windows-msvc.zip`](https://github.com/MrQuality/androgrid-backend/releases/download/v0.0.1-alpha.1/androgrid-v0.0.1-alpha.1-x86_64-pc-windows-msvc.zip) |
| **CLI & Orchestrator** | macOS (Apple Silicon / ARM64) | [`androgrid-v0.0.1-alpha.1-aarch64-apple-darwin.tar.gz`](https://github.com/MrQuality/androgrid-backend/releases/download/v0.0.1-alpha.1/androgrid-v0.0.1-alpha.1-aarch64-apple-darwin.tar.gz) |
| **Sample WASI Workload** | Universal WASM (`wasm32-wasip1`) | [`image-processor.wasm`](https://github.com/MrQuality/androgrid-backend/releases/download/v0.0.1-alpha.1/image-processor.wasm) |
| **Verification Ledger** | SHA-256 Checksums | [`SHA256SUMS.txt`](https://github.com/MrQuality/androgrid-backend/releases/download/v0.0.1-alpha.1/SHA256SUMS.txt) |

### Integrity Verification

Before running downloaded binaries, verify their SHA-256 digests against the published ledger:

```bash
# On Linux / macOS:
sha256sum -c SHA256SUMS.txt

# On Windows PowerShell:
Get-FileHash -Algorithm SHA256 androgrid-worker-v0.0.1-alpha.1.apk
```

---

## 3. Physical Hardware Prerequisites

Verify your test device meets the following minimum specifications:

* **Operating System:** Android 9.0 (API 28) through Android 15 (Vanilla Ice Cream, API 35).
* **RAM:** Minimum 2.0 GB total system RAM (3.0 GB+ recommended for large WASM memory frames).
* **Storage:** Minimum 500 MB free internal flash storage.
* **Camera:** Functional rear camera for scanning ephemeral mTLS enrollment QR codes.
* **Network Connectivity:** Either Wi-Fi (unmetered) or Cellular LTE/5G (metered test mode).
* **Root Access:** Not required. AndroGrid runs hermetically within standard Android application sandboxes.

---

## 4. Step-by-Step Hardware Verification Procedure

Follow this 5-step test sequence to verify cluster execution on your hardware.

### Step 1: Install the Android Worker Application
1. Download `androgrid-worker-v0.0.1-alpha.1.apk` onto your physical phone or tablet (via mobile browser or USB transfer).
2. Tap the downloaded APK to install it. If prompted, allow installations from unknown sources for your browser or file manager.
3. Alternatively, install via ADB from your host workstation:
   ```bash
   adb install -r -g androgrid-worker-v0.0.1-alpha.1.apk
   ```
4. Launch **AndroGrid Worker** from your launcher.
5. When prompted, grant **Notification** permissions (required for typed Foreground Service monitoring under Android 13+) and **Camera** permissions (for QR code pairing).

### Step 2: Launch the Control Plane and Generate a Pairing Token
On your workstation, unpack the native CLI archive and initiate pairing mode:

```bash
# Unpack the downloaded archive (Linux / macOS example):
tar -xzf androgrid-v0.0.1-alpha.1-x86_64-unknown-linux-gnu.tar.gz

# Start pairing session and print ASCII QR code in your terminal:
./androgrid-cli pair --endpoint "https://orchestrator.yourdomain.com:50051"
```

The CLI will display an ephemeral pairing token formatted as an ASCII QR code with a 5-minute expiry window.

### Step 3: Enroll the Device via Camera QR Scanner
1. On your Android device, tap **"Scan Pairing QR"**.
2. Align the camera viewfinder with the QR code displayed in your terminal.
3. Upon detection, the application will:
   * Perform automated mutual TLS (mTLS) enrollment.
   * Generate an ECDSA P-256 private key inside the hardware-backed Android KeyStore.
   * Securely store the cluster root CA certificate.
   * Display **"Node Enrolled & Idle"** on the dashboard.

### Step 4: Dispatch the Canonical Hero Workload
Download `image-processor.wasm` and dispatch a distributed image-inversion task using the CLI:

```bash
# Submit the pre-compiled WASM task to the orchestrator:
./androgrid-cli submit \
  --wasm image-processor.wasm \
  --arg invert \
  --input photo.jpg \
  --output result.png
```

### Step 5: Observe Device Telemetry & Task Execution
Observe the device while the job executes:
1. **Foreground Service Notification:** The worker displays an active notification showing task progress and memory utilization.
2. **Thermal & Battery Protection:** Check the dashboard for battery temperature readings. If the device exceeds 45°C or reaches thermal status `SEVERE`, verify that the worker automatically pauses task claims until cooled down.
3. **Socket Teardown:** Verify that upon task completion, the cellular gRPC socket closes within 200 ms to allow the mobile modem (RRC radio) to return to low-power sleep.

---

## 5. Test Reporting & Bug Filing Protocol

Alpha test feedback directly drives stability fixes for upcoming milestone releases. **All test reports and bug reports are filed through GitHub Issues.**

### Where to File

* **Android Mobile Worker Issues:**
  File at [**androgrid-android/issues/new**](https://github.com/MrQuality/androgrid-android/issues/new)
  * Target topics: APK install errors, camera scanner glitches, crash logs, WorkManager background task killing, thermal throttling false-positives, or UI layout defects.
* **Control Plane, Orchestrator, & CLI Issues:**
  File at [**androgrid-backend/issues/new**](https://github.com/MrQuality/androgrid-backend/issues/new)
  * Target topics: CLI pairing crashes, gRPC mTLS connection timeouts, orchestrator scheduling panics, or corrupted WASM result validation.

---

### Standard Bug Report & Test Execution Format

When filing an issue, please copy, paste, and complete the following markdown template:

```markdown
### Report Type
- [ ] Bug Report (Defect or crash)
- [ ] Hardware Compatibility Report (Successful verification)
- [ ] Performance & Thermal Observation

### Hardware & Environment Profile
- Device Manufacturer & Model: (e.g., Samsung Galaxy S23 Ultra / Google Pixel 7a)
- Android Version & API Level: (e.g., Android 14, API Level 34)
- CPU Architecture: (e.g., arm64-v8a)
- Total System RAM: (e.g., 8 GB)
- Network Connection: (e.g., Home Wi-Fi 6 / 5G Mobile Data)
- Battery Temperature during test: (e.g., 34.2°C)
- Release Version: v0.0.1-alpha.1

### Test Steps Executed
1. Installed APK version v0.0.1-alpha.1 via ADB.
2. Scanned CLI pairing QR code.
3. Dispatched image-processor.wasm with input size 1024x1024.

### Expected Behavior
Task completes successfully, result PNG is reconstructed, and worker returns to idle state.

### Observed Behavior / Failure Description
(Describe what happened, including any error messages or unexpected delays)

### Diagnostic Logs
<details>
<summary>Android Worker Logcat (click to expand)</summary>

```text
(Paste relevant logcat lines here)
```
</details>

<details>
<summary>CLI Terminal Output (click to expand)</summary>

```text
(Paste CLI terminal output with RUST_LOG=debug here)
```
</details>
```

---

### Extracting Diagnostic Logs

#### Extracting Android Worker Logs via ADB
To capture clean diagnostic logs without background system noise, run:

```bash
adb logcat -v time -s AndroGridWorker:V WasmComputeEngine:V WorkManager:I *:S > android_worker.log
```

#### Extracting CLI Verbose Traces
To run the CLI with full debug instrumentation:

```bash
RUST_LOG=debug ./androgrid-cli submit --wasm image-processor.wasm --arg invert --input test.jpg
```

---

## 6. Security & Privacy Notice

> [!CAUTION]
> **Sanitize Diagnostics Before Posting:** Never include private keys, KeyStore aliases, internal password strings, or Cloudflare tunnel access tokens in public GitHub Issues. All logs generated by `adb logcat` using the filter above are strictly sanitized and free of private cryptographic material.
