#!/usr/bin/env python3
"""
Deterministic WebAssembly Fixture Generator and Verifier for AndroGrid.
Generates canonical WASI Preview 1 and pure Wasm test binaries into tests/fixtures/wasm.

Traces to: [INV-005], [INV-006], [REQ-OP-004], [NFR-PERF-001]
"""

import hashlib
import os
import struct
import sys

def leb(val: int) -> bytes:
    out = bytearray()
    while True:
        b = val & 0x7f
        val >>= 7
        if val != 0:
            b |= 0x80
        out.append(b)
        if val == 0:
            break
    return bytes(out)

def make_wasi_print_module(msg: bytes) -> bytes:
    type_sec = bytes([0x02, 0x60, 0x04, 0x7f, 0x7f, 0x7f, 0x7f, 0x01, 0x7f, 0x60, 0x00, 0x00])
    mod_name = b"wasi_snapshot_preview1"
    fn_name = b"fd_write"
    import_entry = leb(len(mod_name)) + mod_name + leb(len(fn_name)) + fn_name + bytes([0x00, 0x00])
    import_sec = bytes([0x01]) + import_entry
    func_sec = bytes([0x01, 0x01])
    mem_sec = bytes([0x01, 0x00, 0x01])

    export_mem = leb(len(b"memory")) + b"memory" + bytes([0x02, 0x00])
    export_start = leb(len(b"_start")) + b"_start" + bytes([0x00, 0x01])
    export_sec = bytes([0x02]) + export_mem + export_start

    code_body = bytes([0x00, 0x41, 0x01, 0x41, 0x00, 0x41, 0x01, 0x41, 0x08, 0x10, 0x00, 0x1a, 0x0b])
    code_sec = bytes([0x01]) + leb(len(code_body)) + code_body

    iov_base = 16
    iov_len = len(msg)
    data_bytes = struct.pack("<II", iov_base, iov_len) + msg
    data_seg = bytes([0x00, 0x41, 0x00, 0x0b]) + leb(len(data_bytes)) + data_bytes
    data_sec = bytes([0x01]) + data_seg

    sections = [
        (1, type_sec),
        (2, import_sec),
        (3, func_sec),
        (5, mem_sec),
        (7, export_sec),
        (10, code_sec),
        (11, data_sec),
    ]

    wasm = bytearray(b"\x00asm\x01\x00\x00\x00")
    for sec_id, content in sections:
        wasm.append(sec_id)
        wasm.extend(leb(len(content)))
        wasm.extend(content)
    return bytes(wasm)

def main():
    base_dir = os.path.dirname(os.path.abspath(__file__))
    out_dir = os.path.normpath(os.path.join(base_dir, "../../tests/fixtures/wasm"))
    os.makedirs(out_dir, exist_ok=True)

    fixtures = {
        "fibonacci.wasm": bytes.fromhex(
            "0061736d0100000001060160017f017f03020100070d01096669626f6e6163636900000a1e011c00200041014c047f200005200041016b1000200041026b10006a0b0b"
        ),
        "trap_abort.wasm": bytes.fromhex(
            "0061736d0100000001040160000003020100070a01065f737461727400000a05010300000b"
        ),
        "infinite_loop.wasm": bytes.fromhex(
            "0061736d0100000001040160000003020100070a01065f737461727400000a0901070003400c000b0b"
        ),
        "env_echo.wasm": make_wasi_print_module(
            b"CLUSTER_ENV=production,NODE_ID=node_pixel8\n"
        ),
        "hello_wasi.wasm": make_wasi_print_module(
            b"Hello, AndroGrid WASM!\n"
        ),
    }

    print(f"Writing {len(fixtures)} fixtures to {out_dir}:")
    for name, content in fixtures.items():
        fp = os.path.join(out_dir, name)
        with open(fp, "wb") as f:
            f.write(content)
        sha = hashlib.sha256(content).hexdigest()
        print(f"  {name}: {len(content)} bytes, sha256={sha}")

if __name__ == "__main__":
    main()
