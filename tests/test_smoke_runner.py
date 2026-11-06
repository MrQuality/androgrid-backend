#!/usr/bin/env python3
"""
Unit and Hermetic Integration Tests for Automated Cluster Smoke Test Harness.

Tests argument parsing, pre-flight device validation, port security audit ([INV-001]),
mTLS rejection assertions ([INV-002]), task status polling ([REQ-OP-004]),
and execution report formatting.

Traces to: [INV-001], [INV-002], [INV-003], [INV-004], [REQ-OP-001], [REQ-OP-004], [NFR-SEC-001]
"""

import sys
import os
import unittest
from pathlib import Path
from unittest.mock import MagicMock, patch, call

# Add tools/smoke to import path
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "tools" / "smoke"))

from test_hardware_cluster import (
    SmokeTestConfig,
    AdbController,
    PkiManager,
    SmokeTestRunner,
    TaskExecutionSummary,
    parse_args,
)


class TestSmokeTestConfigAndArgs(unittest.TestCase):
    """Tests CLI argument parsing and configuration defaults."""

    def test_default_args(self):
        config = parse_args([])
        self.assertEqual(config.port, 50051)
        self.assertTrue(config.use_reverse_adb)
        self.assertEqual(config.orchestrator_mode, "cargo")
        self.assertFalse(config.dry_run)
        self.assertFalse(config.skip_build)

    def test_custom_args(self):
        config = parse_args([
            "--device", "emulator-5554",
            "--port", "50055",
            "--orchestrator-mode", "docker",
            "--dry-run",
            "--skip-build",
            "--host-ip", "10.0.2.2",
            "--no-reverse-adb"
        ])
        self.assertEqual(config.device_serial, "emulator-5554")
        self.assertEqual(config.port, 50055)
        self.assertEqual(config.orchestrator_mode, "docker")
        self.assertTrue(config.dry_run)
        self.assertTrue(config.skip_build)
        self.assertEqual(config.host_ip, "10.0.2.2")
        self.assertFalse(config.use_reverse_adb)


class TestAdbController(unittest.TestCase):
    """Tests ADB controller execution and port parsing logic."""

    @patch("subprocess.run")
    def test_get_connected_devices(self, mock_run):
        mock_run.return_value = MagicMock(
            returncode=0,
            stdout="List of devices attached\nemulator-5554\tdevice product:sdk_gphone64\n192.168.1.101:5555\tdevice\n\n"
        )
        adb = AdbController()
        devices = adb.get_connected_devices()
        self.assertEqual(devices, ["emulator-5554", "192.168.1.101:5555"])

    @patch("subprocess.run")
    def test_check_listening_ports_zero_ports_asserts_inv_001(self, mock_run):
        # Package has no listening ports (only outbound established sockets)
        mock_run.return_value = MagicMock(
            returncode=0,
            stdout="Proto Recv-Q Send-Q Local Address Foreign Address State PID/Program name\ntcp 0 0 10.0.2.15:43210 10.0.2.2:50051 ESTABLISHED 12345/com.androgrid.worker\n"
        )
        adb = AdbController(serial="emulator-5554")
        ports = adb.check_listening_ports("com.androgrid.worker")
        self.assertEqual(ports, [])

    @patch("subprocess.run")
    def test_check_listening_ports_detects_illegal_inbound_port(self, mock_run):
        # Violation of [INV-001]: worker process opened a listening TCP server port
        mock_run.return_value = MagicMock(
            returncode=0,
            stdout="tcp 0 0 0.0.0.0:8080 0.0.0.0:* LISTEN 12345/com.androgrid.worker\n"
        )
        adb = AdbController(serial="emulator-5554")
        ports = adb.check_listening_ports("com.androgrid.worker")
        self.assertIn(8080, ports)

    @patch("subprocess.run")
    def test_get_app_uid(self, mock_run):
        mock_run.return_value = MagicMock(
            returncode=0,
            stdout="userId=10150\n"
        )
        adb = AdbController(serial="emulator-5554")
        uid = adb.get_package_uid("com.androgrid.worker")
        self.assertEqual(uid, "10150")


class TestPkiManager(unittest.TestCase):
    """Tests PKI generation and path resolution."""

    def test_pki_paths(self):
        pki = PkiManager(pki_dir=Path("/tmp/pki_test"))
        paths = pki.get_paths()
        self.assertEqual(paths["ca_cert"], Path("/tmp/pki_test/ca.crt"))
        self.assertEqual(paths["server_cert"], Path("/tmp/pki_test/server.crt"))
        self.assertEqual(paths["server_key"], Path("/tmp/pki_test/server.key"))
        self.assertEqual(paths["client_cert"], Path("/tmp/pki_test/client.crt"))
        self.assertEqual(paths["client_key"], Path("/tmp/pki_test/client.key"))
        self.assertEqual(paths["untrusted_cert"], Path("/tmp/pki_test/untrusted_client.crt"))


class TestSmokeTestRunnerLifecycle(unittest.TestCase):
    """Tests end-to-end smoke test runner orchestration."""

    def setUp(self):
        self.config = SmokeTestConfig(
            device_serial="test-device-01",
            dry_run=True,
            port=50051
        )
        self.runner = SmokeTestRunner(self.config)

    @patch("subprocess.run")
    def test_verify_mtls_rejection_success(self, mock_run):
        # androgrid-cli should fail with non-zero exit code or TLS error when using untrusted cert
        mock_run.return_value = MagicMock(
            returncode=1,
            stderr="transport error: invalid peer certificate: UnknownIssuer"
        )
        rejected = self.runner.verify_mtls_rejection(
            ca_cert=Path("pki/ca.crt"),
            untrusted_cert=Path("pki/untrusted_client.crt"),
            untrusted_key=Path("pki/untrusted_client.key")
        )
        self.assertTrue(rejected)

    @patch("subprocess.run")
    def test_poll_task_status_transitions_to_completed(self, mock_run):
        # Simulate status polling: PENDING -> LEASED -> COMPLETED
        mock_run.side_effect = [
            MagicMock(returncode=0, stdout='{"task_id": "task_1", "state": "TASK_STATE_PENDING"}'),
            MagicMock(returncode=0, stdout='{"task_id": "task_1", "state": "TASK_STATE_LEASED"}'),
            MagicMock(returncode=0, stdout='{"task_id": "task_1", "state": "TASK_STATE_COMPLETED", "exit_code": 0, "stdout": "55"}')
        ]
        summary = self.runner.poll_task_completion(
            task_id="task_1",
            ca_cert=Path("pki/ca.crt"),
            client_cert=Path("pki/client.crt"),
            client_key=Path("pki/client.key"),
            max_attempts=5,
            poll_interval=0.01
        )
        self.assertEqual(summary.state, "TASK_STATE_COMPLETED")
        self.assertEqual(summary.exit_code, 0)
        self.assertEqual(summary.stdout, "55")

    def test_generate_markdown_report_formatting(self):
        summary = TaskExecutionSummary(
            task_id="task_fib_01",
            workload_type="WASM",
            state="TASK_STATE_COMPLETED",
            exit_code=0,
            stdout="55",
            stderr="",
            duration_ms=45.2
        )
        report = self.runner.generate_markdown_report(
            device_serial="emulator-5554",
            device_abi="x86_64",
            inbound_ports=[],
            mtls_enforced=True,
            task_summaries=[summary]
        )
        self.assertIn("# AndroGrid E2E Cluster Smoke Test Report", report)
        self.assertIn("[INV-001]` | Zero Inbound Mobile Ports | PASSED", report)
        self.assertIn("[INV-002]` | mTLS 1.3 Identity Enforcement | PASSED", report)
        self.assertIn("task_fib_01", report)
        self.assertIn("VERDICT: SUCCESS", report)


if __name__ == "__main__":
    unittest.main()
