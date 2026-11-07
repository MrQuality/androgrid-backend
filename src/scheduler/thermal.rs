//! Thermal protection policies for worker nodes.
//!
//! Traces to: AG-SPEC-2026-V2, [REQ-OP-003], [NFR-POW-001], [INV-003]

/// Action to execute when a worker node exceeds thermal bounds.
///
/// Traces to: [NFR-POW-001], [REQ-OP-003]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThermalAction {
    /// Defer task assignment by returning task_available = false (default).
    /// Allows the baseband radio to enter low-power sleep within 200 ms.
    #[default]
    Defer,

    /// Reject task claim with SchedulingError::ThermalThrottled.
    Reject,
}

/// Thermal evaluation policy enforcing battery temperature and OS thermal limits.
///
/// Traces to: [REQ-OP-003], [NFR-POW-001]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThermalPolicy {
    /// Maximum allowable battery temperature in degrees Celsius before throttling.
    /// Default: 38 degrees Celsius per battery longevity and skin temperature bounds.
    pub max_battery_temperature_celsius: i32,

    /// Maximum allowable Android PowerManager thermal status before throttling.
    /// Default: 3 (maps to Android PowerManager.THERMAL_STATUS_SEVERE;
    /// 0=NONE, 1=LIGHT, 2=MODERATE, 3=SEVERE, 4=CRITICAL, 5=EMERGENCY, 6=SHUTDOWN).
    pub max_thermal_status: u32,

    /// Action to execute when a worker node is throttled.
    pub action: ThermalAction,
}

impl Default for ThermalPolicy {
    fn default() -> Self {
        Self {
            max_battery_temperature_celsius: 38,
            max_thermal_status: 3,
            action: ThermalAction::Defer,
        }
    }
}

impl ThermalPolicy {
    /// Constructs a new thermal policy with custom bounds and action.
    ///
    /// # Parameters:
    /// * `max_temp_c` - Maximum allowable battery temperature in degrees Celsius.
    /// * `max_thermal` - Maximum allowable Android thermal status.
    /// * `action` - Action to take when throttled (Defer or Reject).
    pub fn new(max_temp_c: i32, max_thermal: u32, action: ThermalAction) -> Self {
        Self {
            max_battery_temperature_celsius: max_temp_c,
            max_thermal_status: max_thermal,
            action,
        }
    }

    /// Evaluates whether the reported telemetry triggers thermal throttling.
    ///
    /// Per Risk Mitigation 1:
    /// Temperatures <= 0 are treated as unreported or uncalibrated by client hardware
    /// and do NOT trigger temperature throttling. Only temperatures > max_battery_temperature_celsius
    /// trigger throttling.
    ///
    /// Thermal status >= max_thermal_status (when max_thermal_status > 0) triggers throttling.
    ///
    /// # Parameters:
    /// * `battery_temp_celsius` - Reported battery temperature in degrees Celsius.
    /// * `thermal_status` - Reported Android PowerManager thermal status integer.
    ///
    /// # Returns:
    /// * `true` if throttled, `false` otherwise.
    pub fn is_throttled(&self, battery_temp_celsius: i32, thermal_status: u32) -> bool {
        let temp_throttled =
            battery_temp_celsius > self.max_battery_temperature_celsius && battery_temp_celsius > 0;
        let status_throttled =
            self.max_thermal_status > 0 && thermal_status >= self.max_thermal_status;

        temp_throttled || status_throttled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_thermal_policy_default_bounds() {
        let policy = ThermalPolicy::default();
        assert_eq!(policy.max_battery_temperature_celsius, 38);
        assert_eq!(policy.max_thermal_status, 3);
        assert_eq!(policy.action, ThermalAction::Defer);

        // Nominal conditions
        assert!(!policy.is_throttled(25, 0));
        assert!(!policy.is_throttled(38, 2));

        // Elevated temperature > 38°C
        assert!(policy.is_throttled(39, 0));
        assert!(policy.is_throttled(42, 1));

        // Severe thermal status >= 3
        assert!(policy.is_throttled(25, 3));
        assert!(policy.is_throttled(30, 4));

        // Edge case: uncalibrated or 0 temperature must not throttle if status < 3
        assert!(!policy.is_throttled(0, 0));
        assert!(!policy.is_throttled(-10, 0));
        assert!(!policy.is_throttled(0, 2));

        // But uncalibrated temperature with severe status still throttles
        assert!(policy.is_throttled(0, 3));
    }
}
