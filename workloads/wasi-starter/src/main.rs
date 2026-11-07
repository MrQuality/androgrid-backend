//! Canonical WebAssembly (WASI Preview 1) Starter Workload
//!
//! Demonstrates standard edge compute execution inside the AndroGrid unrooted
//! WebAssembly sandbox. Reads environment configuration, executes deterministic
//! compute operations, and outputs structured execution results to stdout.
//!
//! Traces to: AG-SPEC-2026-V2, [INV-006], [REQ-OP-004], [NFR-PERF-001]

use serde::Serialize;
use std::env;
use std::time::Instant;

/// Structured workload execution output returned over stdout.
///
/// Traces to: [REQ-OP-004]
#[derive(Serialize)]
struct WorkloadOutput {
    task_id: String,
    status: &'static str,
    prime_count: usize,
    largest_prime: u64,
    iterations: u64,
    execution_duration_micros: u128,
}

/// Sieve or trial division helper to find primes up to limit.
fn count_primes_up_to(limit: u64) -> (usize, u64) {
    if limit < 2 {
        return (0, 0);
    }
    let mut primes = Vec::new();
    for num in 2..=limit {
        let mut is_prime = true;
        let sqrt = (num as f64).sqrt() as u64;
        for &p in &primes {
            if p > sqrt {
                break;
            }
            if num % p == 0 {
                is_prime = false;
                break;
            }
        }
        if is_prime {
            primes.push(num);
        }
    }
    let count = primes.len();
    let largest = primes.last().copied().unwrap_or(0);
    (count, largest)
}

/// Main entry point for the WASI preview 1 module.
///
/// Traces to: [INV-006], [REQ-OP-004]
fn main() {
    let start_time = Instant::now();

    let task_id = env::var("ANDROGRID_TASK_ID").unwrap_or_else(|_| "task_local_dev".to_string());
    let limit_str = env::var("PRIME_SEARCH_LIMIT").unwrap_or_else(|_| "1000".to_string());
    let limit: u64 = limit_str.parse().unwrap_or(1000);

    let (prime_count, largest_prime) = count_primes_up_to(limit);
    let duration = start_time.elapsed().as_micros();

    let output = WorkloadOutput {
        task_id,
        status: "SUCCESS",
        prime_count,
        largest_prime,
        iterations: limit,
        execution_duration_micros: duration,
    };

    let json_bytes = serde_json::to_string_pretty(&output).expect("Serialization must succeed");
    println!("{json_bytes}");
}
