//! 证据窗口查询可观测指标：scanned_rows / scanned_row_groups / projected_columns /
//! matched_partitions / elapsed_ms / degraded 原因；三条出口（正常/降级/超限）都有指标。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueryMetrics {
    pub scanned_rows: usize,
    pub scanned_row_groups: usize,
    pub projected_columns: usize,
    pub matched_partitions: Vec<String>,
    pub elapsed_ms: u64,
    pub degraded: bool,
    pub degrade_reason: String,
    pub exit_path: String, // normal | degraded | limit_exceeded
    pub interface_code: String,
    pub window_start_ms: i64,
    pub window_end_ms: i64,
}

impl QueryMetrics {
    pub fn new(interface: &str, ws: i64, we: i64) -> Self {
        Self { interface_code: interface.to_string(), window_start_ms: ws, window_end_ms: we,
            exit_path: "normal".into(), ..Default::default() }
    }
    pub fn mark_degraded(&mut self, reason: impl Into<String>) {
        self.degraded = true;
        self.degrade_reason = reason.into();
        self.exit_path = "degraded".into();
    }
    pub fn mark_limit_exceeded(&mut self, reason: impl Into<String>) {
        self.degraded = true;
        self.degrade_reason = reason.into();
        self.exit_path = "limit_exceeded".into();
    }
    pub fn to_json(&self) -> String { serde_json::to_string(self).unwrap_or_else(|_| "{}".into()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_three_exit_paths_produce_metrics() {
        let mut m = QueryMetrics::new("A", 0, 100);
        assert_eq!(m.exit_path, "normal");
        m.mark_degraded("窗口过大");
        assert_eq!(m.exit_path, "degraded");
        assert!(m.degraded);
        let mut n = QueryMetrics::new("A", 0, 100);
        n.mark_limit_exceeded("超内存");
        assert_eq!(n.exit_path, "limit_exceeded");
    }

    #[test]
    fn metrics_serializable_for_ledger() {
        let m = QueryMetrics::new("A", 0, 100);
        let j = m.to_json();
        assert!(j.contains("scanned_rows"));
        assert!(j.contains("matched_partitions"));
    }
}
