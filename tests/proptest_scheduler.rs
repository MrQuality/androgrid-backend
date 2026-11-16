#![allow(deprecated)]

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
            artifact_url: String::new(),
            group_id: String::new(),
            group_epoch: 0,
            node_group_affinity: String::new(),
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
            battery_temperature_celsius: 0,
            thermal_status: 0,
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
            artifact_url: String::new(),
            group_id: String::new(),
            group_epoch: 0,
            node_group_affinity: String::new(),
        };

        scheduler.submit_task(spec).unwrap();
        scheduler.claim_cellular_task_nominal("node-prop", 90, false, 1000).unwrap();

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
                battery_temperature_celsius: 0,
                thermal_status: 0,
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
            battery_temperature_celsius: 0,
            thermal_status: 0,
        };

        scheduler.register_node("node-unrooted-prop".to_string(), caps, "1.0.0".to_string(), 1000).unwrap();

        let caps = scheduler.get_worker_capabilities("node-unrooted-prop").unwrap();
        prop_assert!(!caps.is_rooted);
        prop_assert!(!caps.supported_engines.contains(&(ExecutionEngine::RootShell as i32)));
    }

    /// Invariant [INV-004], [NFR-REL-001]: TaskGroup partition conservation holds across
    /// arbitrary partition counts and Immediate/Drain replacements.
    #[test]
    fn prop_task_group_conservation_and_epoch_fencing(
        p1 in 1usize..16usize,
        p2 in 1usize..16usize,
        use_drain in any::<bool>(),
    ) {
        use androgrid_backend::{
            ReplaceStrategy, ReplaceTaskGroupRequest, SubmitCellularResultRequest, TaskGroupSpec,
            TaskPartitionSpec, TaskTemplate,
        };
        use std::collections::HashMap;

        let scheduler = ClusterScheduler::new();
        let template = TaskTemplate {
            engine: ExecutionEngine::WasmWasi as i32,
            timeout_seconds: 120,
            entrypoint: "_start".to_string(),
            env_vars: HashMap::new(),
            artifact_sha256: "a".repeat(64),
            shell_command: String::new(),
            requires_unmetered: false,
            node_group_affinity: String::new(),
        };

        scheduler
            .submit_task_group(
                TaskGroupSpec {
                    group_id: "prop-grp".to_string(),
                    template: Some(template.clone()),
                    partitions: vec![TaskPartitionSpec::default(); p1],
                },
                1000,
            )
            .unwrap();

        let cell_caps = NodeCapabilities {
            is_rooted: false,
            supported_engines: vec![ExecutionEngine::WasmWasi as i32],
            cpu_cores: 4,
            total_ram_bytes: 4 * 1024 * 1024 * 1024,
            os_version: "14".to_string(),
            abi: "arm64-v8a".to_string(),
            is_metered: true,
            current_transport: NetworkTransportType::NetworkTransportCellularWan as i32,
            max_cellular_task_duration_seconds: 180,
            battery_temperature_celsius: 0,
            thermal_status: 0,
        };
        scheduler
            .register_node("prop-cell".to_string(), cell_caps, "1.0.0".to_string(), 1000)
            .unwrap();
        let claimed = scheduler
            .claim_cellular_task_nominal("prop-cell", 90, false, 10)
            .unwrap()
            .task
            .unwrap();

        let strategy = if use_drain {
            ReplaceStrategy::Drain as i32
        } else {
            ReplaceStrategy::Immediate as i32
        };
        scheduler
            .replace_task_group(
                ReplaceTaskGroupRequest {
                    group_id: "prop-grp".to_string(),
                    expected_epoch: Some(1),
                    template: Some(template),
                    partitions: vec![TaskPartitionSpec::default(); p2],
                    strategy,
                },
                2000,
            )
            .unwrap();

        let st = scheduler.get_task_group_status("prop-grp", false).unwrap();
        let sum = st.pending_tasks
            + st.leased_tasks
            + st.running_tasks
            + st.completed_tasks
            + st.failed_tasks
            + st.timed_out_tasks
            + st.cancelled_tasks
            + st.preempted_tasks;
        prop_assert_eq!(sum, st.total_tasks);
        prop_assert_eq!(st.total_tasks, p2 as u32);
        prop_assert_eq!(st.current_epoch, 2);

        let res = scheduler.submit_cellular_result(&SubmitCellularResultRequest {
            node_id: "prop-cell".to_string(),
            task_id: claimed.task_id,
            run_attempt: claimed.run_attempt,
            exit_code: 0,
            stdout_bytes: Vec::new(),
            stderr_bytes: Vec::new(),
            error_code: String::new(),
            error_message: String::new(),
            execution_duration_ms: 10,
        });
        if use_drain {
            prop_assert!(res.is_ok());
        } else {
            let is_superseded = matches!(res, Err(SchedulingError::GroupSuperseded { .. }));
            prop_assert!(is_superseded);
        }
    }

    #[test]
    fn prop_node_group_affinity_and_drain_invariants(
        assign_to_a in any::<bool>(),
        drain_worker in any::<bool>(),
        target_group_a in any::<bool>(),
    ) {
        use androgrid_backend::{CreateNodeGroupRequest, DrainNodesRequest};
        use std::collections::HashMap;

        let scheduler = ClusterScheduler::new();
        scheduler
            .create_node_group(
                CreateNodeGroupRequest {
                    group_id: "grp-a".to_string(),
                    description: String::new(),
                    label_selector: HashMap::new(),
                    initial_node_ids: Vec::new(),
                },
                1000,
            )
            .unwrap();
        scheduler
            .create_node_group(
                CreateNodeGroupRequest {
                    group_id: "grp-b".to_string(),
                    description: String::new(),
                    label_selector: HashMap::new(),
                    initial_node_ids: Vec::new(),
                },
                1000,
            )
            .unwrap();

        let caps = NodeCapabilities {
            is_rooted: false,
            supported_engines: vec![ExecutionEngine::WasmWasi as i32],
            cpu_cores: 4,
            total_ram_bytes: 4 * 1024 * 1024 * 1024,
            os_version: "14".to_string(),
            abi: "arm64-v8a".to_string(),
            is_metered: false,
            current_transport: NetworkTransportType::NetworkTransportWifiLan as i32,
            max_cellular_task_duration_seconds: 180,
            battery_temperature_celsius: 0,
            thermal_status: 0,
        };
        let reg = scheduler
            .register_node("prop-w1".to_string(), caps, "1.0.0".to_string(), 1000)
            .unwrap();

        let worker_grp = if assign_to_a { "grp-a" } else { "grp-b" };
        scheduler
            .assign_nodes_to_group(
                androgrid_backend::AssignNodesToGroupRequest {
                    group_id: worker_grp.to_string(),
                    node_ids: vec!["prop-w1".to_string()],
                    unassign: false,
                },
                1100,
            )
            .unwrap();

        if drain_worker {
            scheduler
                .drain_nodes(DrainNodesRequest {
                    node_group: String::new(),
                    node_ids: vec!["prop-w1".to_string()],
                    drain: true,
                })
                .unwrap();
        }

        #[allow(deprecated)]
        let spec = TaskSpec {
            task_id: "prop-aff-task".to_string(),
            run_attempt: 0,
            engine: ExecutionEngine::WasmWasi as i32,
            timeout_seconds: 60,
            entrypoint: "_start".to_string(),
            env_vars: HashMap::new(),
            wasm_bytecode: vec![0, 97, 115, 109, 1, 0, 0, 0],
            shell_command: String::new(),
            artifact_sha256: "a".repeat(64),
            requires_unmetered: false,
            artifact_url: String::new(),
            group_id: String::new(),
            group_epoch: 0,
            node_group_affinity: if target_group_a {
                "grp-a".to_string()
            } else {
                "grp-b".to_string()
            },
        };
        scheduler.submit_task(spec).unwrap();

        let sync = scheduler
            .sync_node_state("prop-w1", &reg.session_token, &[], 1200)
            .unwrap();
        if drain_worker || (assign_to_a != target_group_a) {
            prop_assert!(sync.desired_tasks.is_empty());
        } else {
            prop_assert_eq!(sync.desired_tasks.len(), 1);
        }
    }
}
