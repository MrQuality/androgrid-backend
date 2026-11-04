//! Property-Based Testing for Scheduler Invariants
//!
//! Verifies monotonic attempt fencing, capability sanitization, and state progression
//! across randomly generated sequences of inputs.
//!
//! Traces to: AG-SPEC-2026-V2 Section 3.1, [INV-004], [INV-005], [INV-006], [NFR-REL-001]

use androgrid_backend::{
    ClusterScheduler, ExecutionEngine, NetworkTransportType, NodeCapabilities, SchedulingError,
    TaskSpec,
};
use proptest::prelude::*;
use std::collections::HashMap;

prop_compose! {
    fn arb_task_spec()(
        timeout in 1u32..300u32,
        requires_unmetered in any::<bool>(),
        engine_idx in 1i32..4i32,
    ) -> TaskSpec {
        TaskSpec {
            task_id: "prop-task".to_string(),
            run_attempt: 0,
            engine: engine_idx,
            timeout_seconds: timeout,
            entrypoint: "_start".to_string(),
            env_vars: HashMap::new(),
            wasm_bytecode: vec![0, 97, 115, 109, 1, 0, 0, 0],
            shell_command: String::new(),
            artifact_sha256: "sha256-prop".to_string(),
            requires_unmetered,
        }
    }
}

proptest! {
    /// Invariant [INV-004]: Any attempt strictly less than the active attempt must be rejected.
    #[test]
    fn prop_stale_attempts_always_rejected(
        active_attempt in 2u32..500u32,
        delta in 1u32..50u32,
    ) {
        let scheduler = ClusterScheduler::new();
        let caps = NodeCapabilities {
            is_rooted: false,
            supported_engines: vec![ExecutionEngine::WasmWasi as i32],
            cpu_cores: 4,
            total_ram_bytes: 4 * 1024 * 1024 * 1024,
            os_version: "14".to_string(),
            abi: "arm64-v8a".to_string(),
            is_metered: true,
            current_transport: NetworkTransportType::NetworkTransportCellularWan as i32,
            max_cellular_task_duration_seconds: 180,
        };

        scheduler.register_node("node-prop".to_string(), caps, "1.0.0".to_string(), 1000).unwrap();

        let spec = TaskSpec {
            task_id: "prop-task-01".to_string(),
            run_attempt: 0,
            engine: ExecutionEngine::WasmWasi as i32,
            timeout_seconds: 120,
            entrypoint: "_start".to_string(),
            env_vars: HashMap::new(),
            wasm_bytecode: vec![0, 97, 115, 109, 1, 0, 0, 0],
            shell_command: String::new(),
            artifact_sha256: "sha256-prop".to_string(),
            requires_unmetered: false,
        };

        scheduler.submit_task(spec).unwrap();
        scheduler.claim_cellular_task("node-prop", 90, false, 1000).unwrap();

        // Artificially advance attempt by registering and syncing Wi-Fi nodes
        for i in 1..active_attempt {
            let wifi_id = format!("wifi-sync-{}", i);
            let wifi_caps = NodeCapabilities {
                is_rooted: false,
                supported_engines: vec![ExecutionEngine::WasmWasi as i32],
                cpu_cores: 4,
                total_ram_bytes: 4 * 1024 * 1024 * 1024,
                os_version: "14".to_string(),
                abi: "arm64-v8a".to_string(),
                is_metered: false,
                current_transport: NetworkTransportType::NetworkTransportWifiLan as i32,
                max_cellular_task_duration_seconds: 180,
            };
            let reg = scheduler.register_node(wifi_id.clone(), wifi_caps, "1.0.0".to_string(), 2000).unwrap();
            let _ = scheduler.sync_node_state(&wifi_id, &reg.session_token, &[], 2000);
        }

        let current_active = scheduler.get_active_attempt("prop-task-01").unwrap();
        prop_assert!(current_active >= 2);

        // A stale attempt less than current_active
        let stale_attempt = current_active.saturating_sub(delta).max(1);
        if stale_attempt < current_active {
            let req = androgrid_backend::SubmitCellularResultRequest {
                node_id: "node-prop".to_string(),
                task_id: "prop-task-01".to_string(),
                run_attempt: stale_attempt,
                exit_code: 0,
                stdout_bytes: Vec::new(),
                stderr_bytes: Vec::new(),
                error_code: String::new(),
                error_message: String::new(),
                execution_duration_ms: 100,
            };
            let res = scheduler.submit_cellular_result(&req);

            let is_rejected = matches!(
                res,
                Err(SchedulingError::AttemptStale { .. }) | Err(SchedulingError::AlreadyFinalized(..))
            );
            prop_assert!(is_rejected);
        }
    }

    /// Invariant [INV-006]: Unrooted node registration NEVER retains RootShell engine.
    #[test]
    fn prop_unrooted_registration_never_includes_root_shell(
        has_root_in_input in any::<bool>(),
        cpu_cores in 1u32..32u32,
    ) {
        let scheduler = ClusterScheduler::new();
        let mut engines = vec![ExecutionEngine::WasmWasi as i32, ExecutionEngine::ToyboxUser as i32];
        if has_root_in_input {
            engines.push(ExecutionEngine::RootShell as i32);
        }

        let caps = NodeCapabilities {
            is_rooted: false,
            supported_engines: engines,
            cpu_cores,
            total_ram_bytes: 4 * 1024 * 1024 * 1024,
            os_version: "14".to_string(),
            abi: "arm64-v8a".to_string(),
            is_metered: true,
            current_transport: NetworkTransportType::NetworkTransportCellularWan as i32,
            max_cellular_task_duration_seconds: 180,
        };

        scheduler.register_node("node-unrooted-prop".to_string(), caps, "1.0.0".to_string(), 1000).unwrap();

        let caps = scheduler.get_worker_capabilities("node-unrooted-prop").unwrap();
        prop_assert!(!caps.is_rooted);
        prop_assert!(!caps.supported_engines.contains(&(ExecutionEngine::RootShell as i32)));
    }
}
