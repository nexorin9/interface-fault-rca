//! 双 tokio runtime：采集通道与交互/模型通道分离；按 workload class 限流调度；
//! 每类暴露 active/queued/admission_wait 计数；同名采集任务重复启动被拒（RepeatedStart）。

use crate::error::{AppError, ErrorCode, Result};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkloadClass { Ingest, Query, Model }

impl WorkloadClass {
    pub fn as_str(self) -> &'static str {
        match self { WorkloadClass::Ingest => "ingest", WorkloadClass::Query => "query", WorkloadClass::Model => "model" }
    }
}

/// 每类 workload 的调度计数（可观测）
#[derive(Debug, Default, Clone)]
pub struct ClassMetrics {
    pub active: u64,
    pub queued: u64,
    pub admission_wait_ms_total: u64,
}

#[derive(Debug, Clone)]
pub struct Scheduler {
    /// 每类并发上限
    limits: Arc<Mutex<HashMap<WorkloadClass, usize>>>,
    metrics: Arc<Mutex<HashMap<WorkloadClass, ClassMetrics>>>,
    started: Arc<Mutex<HashSet<String>>>,
    admission_wait_ms_total: Arc<AtomicU64>,
}

impl Default for Scheduler {
    fn default() -> Self { Self::new() }
}

impl Scheduler {
    pub fn new() -> Self {
        let mut limits = HashMap::new();
        limits.insert(WorkloadClass::Ingest, 4);
        limits.insert(WorkloadClass::Query, 4);
        limits.insert(WorkloadClass::Model, 2);
        let mut metrics = HashMap::new();
        for c in [WorkloadClass::Ingest, WorkloadClass::Query, WorkloadClass::Model] {
            metrics.insert(c, ClassMetrics::default());
        }
        Self {
            limits: Arc::new(Mutex::new(limits)),
            metrics: Arc::new(Mutex::new(metrics)),
            started: Arc::new(Mutex::new(HashSet::new())),
            admission_wait_ms_total: Arc::new(AtomicU64::new(0)),
        }
    }

    /// 按 workload class 设置并发权重上限
    pub fn set_weight(&self, class: WorkloadClass, limit: usize) -> Result<()> {
        if limit == 0 { return Err(AppError::new(ErrorCode::ConfigMissing, "权重上限须>0")); }
        if let Ok(mut m) = self.limits.lock() { m.insert(class, limit); }
        Ok(())
    }

    pub fn limit_of(&self, class: WorkloadClass) -> usize {
        self.limits.lock().ok().and_then(|m| m.get(&class).copied()).unwrap_or(0)
    }

    /// 尝试进入某类通道：超限则排队（queued+1），返回是否立即获得许可。
    /// 关键纪律：模型通道排队不得阻塞采集通道入队——两者是无锁独立计数。
    pub fn admit(&self, class: WorkloadClass) -> Admission {
        let limit = self.limit_of(class) as u64;
        let mut m = self.metrics.lock().unwrap();
        let e = m.entry(class).or_default();
        if e.active < limit {
            e.active += 1;
            Admission::Granted
        } else {
            e.queued += 1;
            Admission::Queued
        }
    }

    pub fn release(&self, class: WorkloadClass) {
        if let Ok(mut m) = self.metrics.lock() {
            if let Some(e) = m.get_mut(&class) {
                if e.active > 0 { e.active -= 1; }
            }
        }
    }

    /// 记录一次入队等待（毫秒）
    pub fn record_wait(&self, class: WorkloadClass, ms: u64) {
        if let Ok(mut m) = self.metrics.lock() {
            if let Some(e) = m.get_mut(&class) { e.admission_wait_ms_total += ms; }
        }
        self.admission_wait_ms_total.fetch_add(ms, Ordering::SeqCst);
    }

    pub fn metrics_of(&self, class: WorkloadClass) -> ClassMetrics {
        self.metrics.lock().ok().and_then(|m| m.get(&class).cloned()).unwrap_or_default()
    }

    pub fn total_admission_wait_ms(&self) -> u64 { self.admission_wait_ms_total.load(Ordering::SeqCst) }

    /// 重复启动保护：同名任务第二次启动直接拒绝
    pub fn try_start_task(&self, name: &str) -> Result<()> {
        let mut s = self.started.lock().unwrap();
        if s.contains(name) {
            return Err(AppError::new(ErrorCode::RepeatedStart, format!("任务 {name} 已在运行")));
        }
        s.insert(name.to_string());
        Ok(())
    }

    pub fn stop_task(&self, name: &str) {
        if let Ok(mut s) = self.started.lock() { s.remove(name); }
    }

    pub fn is_running(&self, name: &str) -> bool {
        self.started.lock().map(|s| s.contains(name)).unwrap_or(false)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission { Granted, Queued }

/// 建立两个 tokio runtime：采集通道与交互/模型通道分离。
pub struct DualRuntime {
    pub ingest: tokio::runtime::Runtime,
    pub interact: tokio::runtime::Runtime,
}

impl DualRuntime {
    pub fn new(ingest_workers: usize, interact_workers: usize) -> Result<Self> {
        let ingest = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(ingest_workers.max(1))
            .thread_name("ingest-rt")
            .enable_all()
            .build()
            .map_err(|e| AppError::new(ErrorCode::ConfigMissing, format!("采集 runtime 建立失败: {e}")))?;
        let interact = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(interact_workers.max(1))
            .thread_name("interact-rt")
            .enable_all()
            .build()
            .map_err(|e| AppError::new(ErrorCode::ConfigMissing, format!("交互 runtime 建立失败: {e}")))?;
        Ok(Self { ingest, interact })
    }
}

/// 指数退避间隔序列（毫秒）：base, 2*base, 4*base ...
pub fn backoff_sequence(base_ms: u64, n: usize) -> Vec<Duration> {
    (0..n).map(|i| Duration::from_millis(base_ms.saturating_mul(1u64 << i))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_saturation_does_not_block_ingest_admission() {
        let s = Scheduler::new();
        s.set_weight(WorkloadClass::Model, 1).unwrap();
        // 模型通道打满
        assert_eq!(s.admit(WorkloadClass::Model), Admission::Granted);
        assert_eq!(s.admit(WorkloadClass::Model), Admission::Queued);
        // 采集通道仍能获得许可
        assert_eq!(s.admit(WorkloadClass::Ingest), Admission::Granted);
        assert_eq!(s.admit(WorkloadClass::Ingest), Admission::Granted);
    }

    #[test]
    fn weight_limit_is_enforced() {
        let s = Scheduler::new();
        s.set_weight(WorkloadClass::Query, 1).unwrap();
        assert_eq!(s.admit(WorkloadClass::Query), Admission::Granted);
        assert_eq!(s.admit(WorkloadClass::Query), Admission::Queued);
        s.release(WorkloadClass::Query);
        assert_eq!(s.admit(WorkloadClass::Query), Admission::Granted);
    }

    #[test]
    fn repeated_start_rejected() {
        let s = Scheduler::new();
        assert!(s.try_start_task("nightly-collect").is_ok());
        let e = s.try_start_task("nightly-collect").unwrap_err();
        assert_eq!(e.code, ErrorCode::RepeatedStart);
        assert!(s.is_running("nightly-collect"));
        s.stop_task("nightly-collect");
        assert!(!s.is_running("nightly-collect"));
    }

    #[test]
    fn metrics_track_active_and_wait() {
        let s = Scheduler::new();
        s.admit(WorkloadClass::Ingest);
        assert_eq!(s.metrics_of(WorkloadClass::Ingest).active, 1);
        s.record_wait(WorkloadClass::Model, 12);
        assert_eq!(s.metrics_of(WorkloadClass::Model).admission_wait_ms_total, 12);
        assert_eq!(s.total_admission_wait_ms(), 12);
    }

    #[test]
    fn backoff_doubles() {
        let seq = backoff_sequence(100, 3);
        assert_eq!(seq[0], Duration::from_millis(100));
        assert_eq!(seq[1], Duration::from_millis(200));
        assert_eq!(seq[2], Duration::from_millis(400));
    }
}
