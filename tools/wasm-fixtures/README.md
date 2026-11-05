# Canonical WebAssembly (WASI) Test Fixtures

## Overview
This directory contains canonical WebAssembly (WASI Preview 1) workload fixtures for validating edge compute task execution, scheduling, SHA-256 payload integrity verification, and operator CLI submission in AndroGrid.

Traces to: [INV-005], [INV-006], [REQ-OP-004], [NFR-PERF-001]

---

## Fixture Inventory & Cryptographic Hashes

| Fixture File | Target Architecture | Size | SHA-256 Digest | Purpose |
| :--- | :--- | :--- | :--- | :--- |
| `fibonacci.wasm` | `wasm32-wasip1` / core | 67 B | `2986333e9b06b321fefdec5227576a9c695c95a3a3fd442e0b20196d3e6a152b` | Recursive Fibonacci compute calculation (`fibonacci(10) == 55`). |
| `env_echo.wasm` | `wasm32-wasip1` | 165 B | `44d49882427ac4fdf46c27abb3ed22a61a3e8024f9186afeb44ecd8f9f224720` | WASI standard stream verification via `fd_write` (prints environment variables). |
| `trap_abort.wasm` | `wasm32-wasip1` / core | 37 B | `187e8d9bf0a3454a6765f0eb83f1398f350a5c053bd3907b430dd05f4c91134f` | Deliberate `unreachable` trap to verify runtime trap isolation (`WASM_TRAP`). |
| `infinite_loop.wasm` | `wasm32-wasip1` / core | 41 B | `4ed2cef087a79f7abf368e3903d56c80dd716912cb78b7c9df8a9a8046c5f3a9` | Non-terminating busy loop to verify timeout watchdog termination (`TIMEOUT`). |
| `hello_wasi.wasm` | `wasm32-wasip1` | 153 B | `a3cb46ccd67461cd02e6d113da021c8e8268285357eb947b003428f6c972338d` | Standard WASI Preview 1 stdout greeting fixture. |

---

## Constraints & Architectural Invariants
1. **[NFR-PERF-001] Frame Boundaries:** All test workload binaries remain well under the 16 MB message payload limit (< 1 KB each).
2. **[INV-006] Sandbox Boundary:** These artifacts run hermetically inside the unrooted Android WebAssembly runtime sandbox.
3. **Typographic Standard:** Strictly zero em dashes (U+2014) or en dashes (U+2013).
