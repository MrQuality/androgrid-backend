# Workload Developer Guide: Compiling Any Language to WASI

This guide provides instructions for software developers and data engineers to compile source code written in **Rust, Go, C/C++, Python, and JavaScript** into standalone WebAssembly/WASI Preview 1 (`.wasm`) binaries that run hermetically on AndroGrid mobile worker nodes.

Traceability: `[REQ-OP-004]`, `[INV-006]`, `[REQ-PUB-001]`, `[NFR-PERF-001]`, `[NFR-POW-001]`.

---

## Important Notice: Interim Toolchain Workflow

<div class="callout callout-warning">
  <strong>Interim Development Step:</strong> The manual compiler invocations documented below represent an interim workflow while the automated build engine is undergoing formal architecture review. A comprehensive build harness (<code>androgrid-cli build</code>) that compiles source directories automatically inside rootless Podman containers is currently tracked in <a href="https://github.com/MrQuality/androgrid-backend/issues/22" target="_blank">RFC Issue #22 &nearr;</a>. Until that engine is released, use the language-specific recipes below to emit your <code>.wasm</code> payloads.
</div>

---

## Architectural Context: Why WASM is Required

Unlike traditional cloud clusters where worker servers execute native x86_64 Linux binaries, distributed consumer mobile devices enforce strict platform security:

1. **Heterogeneous CPU Architectures:** Mobile worker nodes in the cluster feature different architectures: 64-bit ARM (`arm64-v8a`), 32-bit ARM (`armeabi-v7a`), or x86_64 emulators. A native Linux binary compiled for x86_64 cannot execute on an ARM phone. WebAssembly provides universal bytecode that executes deterministically across all phone chips.
2. **Android Security & `noexec` Restrictions:** Android mounts application storage with the `noexec` flag and enforces strict SELinux domain policies. Android 10+ (API 29+) forbids executing dynamic shared libraries (`.so`) or ELF binaries from writable app storage (`[REQ-VER-003]`). Any attempt by an unrooted app to `fork()` or `execve()` external native binaries results in an immediate `PermissionDenied` kernel kill.
3. **In-Memory Sandbox Boundary (`[INV-006]`):** WebAssembly executes in memory within the in-process **Chicory** WASI interpreter, allowing safe computation on unrooted devices without exposing host filesystem or socket permissions.

---

## Direct Shell Execution (Rooted / Toybox Mode)

If your worker nodes are rooted or you only require basic shell utilities, you **do not** need WebAssembly. The control plane supports raw shell execution directly:

```bash
# Execute unprivileged shell commands on unrooted Android toybox:
androgrid-cli submit \
  --shell "uptime; cat /proc/cpuinfo | grep 'model name' | head -n 1" \
  --timeout 30

# Execute privileged shell commands on rooted worker hardware ([INV-006]):
androgrid-cli submit \
  --shell "iptables -L -n" \
  --root \
  --timeout 30
```

---

## Multi-Language Compilation Recipes

### 1. Rust (`wasm32-wasip1`)

Rust provides tier-1 support for WebAssembly System Interface (WASI). It produces highly optimized, low-memory binaries ideal for mobile edge nodes.

#### Setup & Compilation
```bash
# 1. Install the official WASI Preview 1 target:
rustup target add wasm32-wasip1

# 2. Build your project in release mode:
cargo build --target wasm32-wasip1 --release
```

The output binary is generated at:
`target/wasm32-wasip1/release/<PROJECT_NAME>.wasm`

#### Optional: Bytecode Optimization
To reduce binary size over cellular networks, strip debug symbols using `wasm-opt`:
```bash
wasm-opt -O3 target/wasm32-wasip1/release/<PROJECT_NAME>.wasm -o optimized.wasm
```

---

### 2. Go (TinyGo & Standard Go)

#### Option A: TinyGo (Recommended for Mobile)
TinyGo generates lightweight WebAssembly binaries (often under 200 KB) with minimal memory overhead:

```bash
# Compile with WASI target and size optimizations:
tinygo build -target=wasi -opt=2 -o app.wasm main.go
```

#### Option B: Standard Go (Go 1.21+)
Standard Go supports WASI Preview 1 directly:

```bash
GOOS=wasip1 GOARCH=wasm go build -o app.wasm main.go
```

---

### 3. C / C++ (Using `wasi-sdk`)

Use the official `wasi-sdk` clang compiler to emit standalone WASI modules:

```bash
# Install wasi-sdk, then compile:
/opt/wasi-sdk/bin/clang \
  --target=wasm32-wasi \
  -O3 \
  -o app.wasm \
  main.c
```

---

### 4. Python Scripts

Python scripts cannot be converted to standalone machine code directly. Instead, execute Python scripts inside the WASI sandbox using one of two approaches:

#### Approach A: Pre-Packaged CPython WASI Interpreter
Use a pre-compiled `python.wasm` binary:
```bash
# Package your script alongside the WASI Python interpreter:
androgrid-cli submit \
  --wasm ./tools/runtimes/python.wasm \
  --env PYTHON_SCRIPT="print('Hello from Python on Android!')" \
  --timeout 30
```

#### Approach B: Single-Binary Bundling via `componentize-py`
`componentize-py` bundles Python source code and the embedded interpreter into a single self-contained `.wasm` binary:
```bash
# Install tool:
pip install componentize-py

# Componentize Python source into standalone WASM:
componentize-py -d ./app -m main componentize -o app.wasm
```

---

### 5. JavaScript / TypeScript (Using `javy`)

Shopify's **Javy** tool bundles JavaScript code with an embedded QuickJS engine into a standalone WebAssembly binary:

```bash
# Install Javy CLI:
cargo install javy-cli

# Compile JS to standalone WASI binary:
javy compile -o app.wasm script.js
```

---

## Packaging & Submitting the Compiled WASM Binary

Once you have generated your `app.wasm` binary:

### 1. Validate WASI Compatibility (`androgrid-cli pack`)
```bash
androgrid-cli pack \
  --wasm ./app.wasm \
  --entrypoint _start \
  --output manifest.json
```

Output:
```text
[+] WebAssembly module validated successfully: wasi_snapshot_preview1
    Artifact SHA-256: 3c79a8bc43f55de17e88ef22ad71e860951a8bc4953c52e42efaa7be04294d1a
    Size:             182410 bytes (178 KB)
    Entrypoint:       _start
[+] Manifest written to manifest.json
```

### 2. Submit to the Cluster (`androgrid-cli submit`)
```bash
androgrid-cli submit \
  --wasm ./app.wasm \
  --timeout 45 \
  --env BATCH_SIZE=100
```

The CLI automatically indexes the bytecode in content-addressed storage and dispatches the task lease across connected Android worker nodes.
