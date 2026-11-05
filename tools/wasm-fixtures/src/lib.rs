//! Reference Rust Source Definitions for Canonical WebAssembly Fixtures.
//!
//! Contains reference source code compiled to `wasm32-wasip1` and pure Wasm.
//!
//! Traces to: [INV-005], [INV-006], [REQ-OP-004], [NFR-PERF-001]

/// Calculates the Nth Fibonacci number recursively.
#[no_mangle]
pub extern "C" fn fibonacci(n: i32) -> i32 {
    if n <= 1 {
        n
    } else {
        fibonacci(n - 1) + fibonacci(n - 2)
    }
}

/// Deliberate runtime trap fixture invoking unreachable instruction.
#[no_mangle]
pub extern "C" fn trap_abort() {
    #[cfg(target_arch = "wasm32")]
    core::arch::wasm32::unreachable();
    #[cfg(not(target_arch = "wasm32"))]
    panic!("Deliberate trap triggered");
}

/// Runaway execution fixture executing an infinite loop.
#[no_mangle]
pub extern "C" fn infinite_loop() -> ! {
    loop {}
}
