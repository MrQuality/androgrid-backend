//! Concurrency Stress Spike: Atomic CAS Consensus & Race Resolution
//!
//! Simulates high-contention concurrent worker submissions and preemption races
//! to prove zero double-commits and deterministic monotonic consensus under load.
//!
//! Traces to: AG-SPEC-2026-V2 Section 3.1, Section 7, [INV-004], [INV-005], [NFR-REL-001]

use androgrid_backend::{
    ClusterScheduler, ExecutionEngine, NetworkTransportType, NodeCapabilities, TaskSpec, TaskState,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

fn create_caps(is_metered: bool) -> NodeCapabilities {
    NodeCapabilities {
        is_rooted: false,
        supported_engines: vec![ExecutionEngine::WasmWasi as i32],
        cpu_cores: 8,
        total_ram_bytes: 8 * 1024 * 1024 * 1024,
        os_version: "14".to_string(),
        abi: "arm64-v8a".to_string(),
        is_metered,
        current_transport: if is_metered {
            NetworkTransportType::NetworkTransportCellularWan as i32
        } else {
            NetworkTransportType::NetworkTransportWifiLan as i32
        },
        max_cellular_task_duration_seconds: 180,
    }
}

/// Stress tests concurrent attempts to submit results for the same task.
/// 50 concurrent worker threads race to commit results for task "race-task-01".
/// Exactly ONE commit must succeed; 49 must be rejected.
///
/// Traces to: [INV-004], [NFR-REL-001]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn spike_concurrent_result_submissions_zero_double_commit() {
    let scheduler = Arc::new(ClusterScheduler::new());

    // Submit task
    let task_id = "race-task-01".to_string();
    let spec = TaskSpec {
        task_id: task_id.clone(),
        run_attempt: 0,
        engine: ExecutionEngine::WasmWasi as i32,
        timeout_seconds: 120,
        entrypoint: "_start".to_string(),
        env_vars: HashMap::new(),
        wasm_bytecode: vec![0, 97, 115, 109, 1, 0, 0, 0],
        shell_command: String::new(),
        artifact_sha256: "sha256-abc".to_string(),
        requires_unmetered: false,
    };
    scheduler.submit_task(spec).unwrap();

    // Register 50 cellular nodes
    let num_workers = 50;
    for i in 0..num_workers {
        let node_id = format!("worker-{:03}", i);
        scheduler
            .register_node(node_id, create_caps(true), "1.0.0".to_string(), 1000)
            .unwrap();
    }

    // Node 0 claims the task with run_attempt = 1
    let claim = scheduler
        .claim_cellular_task("worker-000", 90, false, 1000)
        .unwrap();
    assert!(claim.task_available);

    // Spawn 50 tasks racing to submit result for attempt 1
    let committed_count = Arc::new(AtomicUsize::new(0));
    let rejected_count = Arc::new(AtomicUsize::new(0));

    let start_time = Instant::now();
    let mut handles = Vec::new();

    for i in 0..num_workers {
        let sched = Arc::clone(&scheduler);
        let committed = Arc::clone(&committed_count);
        let rejected = Arc::clone(&rejected_count);
        let tid = task_id.clone();
        let node_id = format!("worker-{:03}", i);

        let handle = tokio::spawn(async move {
            let req = androgrid_backend::SubmitCellularResultRequest {
                node_id: node_id.clone(),
                task_id: tid,
                run_attempt: 1, // All attempt the active attempt 1
                exit_code: 0,
                stdout_bytes: format!("Output from {}", node_id).into_bytes(),
                stderr_bytes: Vec::new(),
                error_code: String::new(),
                error_message: String::new(),
                execution_duration_ms: 1200,
            };
            let res = sched.submit_cellular_result(&req);

            match res {
                Ok(_) => {
                    committed.fetch_add(1, Ordering::SeqCst);
                }
                Err(_) => {
                    rejected.fetch_add(1, Ordering::SeqCst);
                }
            }
        });
        handles.push(handle);
    }

    for h in handles {
        h.await.unwrap();
    }
    let elapsed = start_time.elapsed();

    // Invariant Assertions:
    // Exactly 1 commit succeeded, exactly 49 were rejected
    assert_eq!(
        committed_count.load(Ordering::SeqCst),
        1,
        "Exactly one worker must commit the task"
    );
    assert_eq!(
        rejected_count.load(Ordering::SeqCst),
        num_workers - 1,
        "All other concurrent submissions must be rejected"
    );
    assert_eq!(
        scheduler.get_task_state(&task_id),
        Some(TaskState::Completed),
        "Final state must be COMPLETED"
    );

    println!(
        "Concurrency Spike: 50 threads completed in {:?}. 1 committed, 49 rejected. Zero double-commits.",
        elapsed
    );
}

/// Stress tests speculative dual-assignment preemption under concurrent execution.
/// A cellular worker and a Wi-Fi worker race concurrently.
/// Wi-Fi worker has run_attempt = 2, cellular worker has run_attempt = 1.
///
/// Traces to: [INV-004], [INV-005], [NFR-REL-001]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn spike_speculative_preemption_concurrent_race() {
    let scheduler = Arc::new(ClusterScheduler::new());

    let task_id = "preemption-race-task".to_string();
    let spec = TaskSpec {
        task_id: task_id.clone(),
        run_attempt: 0,
        engine: ExecutionEngine::WasmWasi as i32,
        timeout_seconds: 120,
        entrypoint: "_start".to_string(),
        env_vars: HashMap::new(),
        wasm_bytecode: vec![0, 97, 115, 109, 1, 0, 0, 0],
        shell_command: String::new(),
        artifact_sha256: "sha256-xyz".to_string(),
        requires_unmetered: false,
    };
    scheduler.submit_task(spec).unwrap();

    // Register cellular and Wi-Fi worker
    scheduler
        .register_node(
            "cell-node".to_string(),
            create_caps(true),
            "1.0.0".to_string(),
            1000,
        )
        .unwrap();
    let wifi_reg = scheduler
        .register_node(
            "wifi-node".to_string(),
            create_caps(false),
            "1.0.0".to_string(),
            1000,
        )
        .unwrap();

    // Cellular worker claims task -> attempt 1
    scheduler
        .claim_cellular_task("cell-node", 80, false, 1000)
        .unwrap();

    // Wi-Fi worker syncs -> preempts with attempt 2
    let sync = scheduler
        .sync_node_state("wifi-node", &wifi_reg.session_token, &[], 1050000)
        .unwrap();
    assert_eq!(sync.desired_tasks.len(), 1);
    assert_eq!(sync.desired_tasks[0].run_attempt, 2);

    // Both workers submit their results concurrently
    let sched1 = Arc::clone(&scheduler);
    let tid1 = task_id.clone();
    let cell_handle = tokio::spawn(async move {
        let req = androgrid_backend::SubmitCellularResultRequest {
            node_id: "cell-node".to_string(),
            task_id: tid1,
            run_attempt: 1, // Stale attempt 1
            exit_code: 0,
            stdout_bytes: b"cell result".to_vec(),
            stderr_bytes: Vec::new(),
            error_code: String::new(),
            error_message: String::new(),
            execution_duration_ms: 45000,
        };
        sched1.submit_cellular_result(&req)
    });

    let sched2 = Arc::clone(&scheduler);
    let tid2 = task_id.clone();
    let wifi_handle = tokio::spawn(async move {
        let req = androgrid_backend::SubmitCellularResultRequest {
            node_id: "wifi-node".to_string(),
            task_id: tid2,
            run_attempt: 2, // Higher attempt 2
            exit_code: 0,
            stdout_bytes: b"wifi result".to_vec(),
            stderr_bytes: Vec::new(),
            error_code: String::new(),
            error_message: String::new(),
            execution_duration_ms: 5000,
        };
        sched2.submit_cellular_result(&req)
    });

    let (cell_res, wifi_res) = tokio::join!(cell_handle, wifi_handle);
    let cell_result = cell_res.unwrap();
    let wifi_result = wifi_res.unwrap();

    // Wi-Fi attempt (2) must be accepted
    assert!(
        wifi_result.is_ok(),
        "Wi-Fi worker with attempt 2 must be accepted"
    );

    // Cellular attempt (1) must be rejected with AttemptStale or AlreadyFinalized
    assert!(
        cell_result.is_err(),
        "Cellular worker with stale attempt 1 must be rejected"
    );

    // Verify committed result belongs to Wi-Fi attempt 2
    let result = scheduler.get_task_result(&task_id).unwrap();
    assert_eq!(result.committed_attempt, 2);
    assert_eq!(result.committed_by_node_id, "wifi-node");
}
