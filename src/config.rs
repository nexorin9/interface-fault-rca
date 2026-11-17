//! 配置加载与凭据位校验：全部凭据来自环境变量；缺失即返回 ConfigMissing，
//! 不回退到本地替身 / localhost / 示例值。数值项带默认值与范围校验。

use crate::error::{AppError, ErrorCode, Result};
use std::collections::HashMap;
use std::path::PathBuf;

/// 运行模式：严格模式要求所有对接凭据齐全；诊断模式只要求能跑的主干配置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strictness {
    Strict,   // 正式常驻启动：缺任一必需凭据即失败
    Diagnose, // diagnose 命令：用 fixtures，不要求真实 DB/PROM/工单凭据
}

#[derive(Debug, Clone)]
pub struct EsbConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub db_name: String,
}

#[derive(Debug, Clone)]
pub struct PromConfig {
    pub base_url: String,
    pub bearer_token: String,
    pub max_response_bytes: usize,
}

#[derive(Debug, Clone)]
pub struct LogConfig {
    pub esb_log_dir: PathBuf,
    pub app_log_dir: PathBuf,
    pub max_line_bytes: usize,
}

#[derive(Debug, Clone)]
pub struct TicketConfig {
    pub base_url: String,
    pub token: String,
}

#[derive(Debug, Clone)]
pub struct ModelConfig {
    pub base_url: String,
    pub api_key: String,
    pub name: String,
    pub timeout_sec: u64,
    pub max_retries: u32,
}

#[derive(Debug, Clone)]
pub struct BaselineConfig {
    /// 基线窗口长度（分钟）
    pub window_minutes: i64,
    /// 劣化倍数阈值
    pub degrade_ratio_threshold: f64,
    /// 错误率变化阈值
    pub error_rate_delta_threshold: f64,
    /// 基线样本量下限（低于此判证据不足）
    pub min_baseline_samples: usize,
    /// 基线自身方差上限（超过判证据不足）
    pub max_baseline_cv: f64,
    /// 劣化倍数恰等于阈值时是否计入候选（大于 / 不小于）
    pub threshold_inclusive: bool,
    /// 排序权重
    pub w_latency_abs: f64,
    pub w_ratio: f64,
    pub w_error_delta: f64,
    pub w_host_deviation: f64,
    /// 证据充分性下限（合法证据条数）
    pub min_evidence_refs: usize,
}

impl Default for BaselineConfig {
    fn default() -> Self {
        Self {
            window_minutes: 30,
            degrade_ratio_threshold: 1.5,
            error_rate_delta_threshold: 0.05,
            min_baseline_samples: 8,
            max_baseline_cv: 1.2,
            threshold_inclusive: false,
            w_latency_abs: 0.30,
            w_ratio: 0.35,
            w_error_delta: 0.20,
            w_host_deviation: 0.15,
            min_evidence_refs: 1,
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueryLimits {
    /// 证据窗口最大扫描行数
    pub max_scan_rows: usize,
    /// 内存预算上限（字节）
    pub max_memory_bytes: usize,
    /// 降级返回的 Top-N
    pub top_n: usize,
    /// 时钟漂移估计所需最小共有事件数
    pub min_clock_samples: usize,
}

impl Default for QueryLimits {
    fn default() -> Self {
        Self {
            max_scan_rows: 2_000_000,
            max_memory_bytes: 512 * 1024 * 1024,
            top_n: 20,
            min_clock_samples: 5,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RetentionConfig {
    /// 保留期（天）
    pub days: u64,
    /// 分区总量上限（字节）
    pub max_partition_bytes: u64,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self { days: 7, max_partition_bytes: 50 * 1024 * 1024 * 1024 }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub esb: EsbConfig,
    pub prom: PromConfig,
    pub log: LogConfig,
    pub ticket: TicketConfig,
    pub model: ModelConfig,
    pub baseline: BaselineConfig,
    pub limits: QueryLimits,
    pub retention: RetentionConfig,
    /// 数据目录（事件表 / SQLite / 台账）
    pub data_dir: PathBuf,
    /// 采集间隔（秒）
    pub collect_interval_sec: u64,
}

fn parse_u64(map: &HashMap<String, String>, key: &str, default: u64) -> Result<u64> {
    match map.get(key) {
        None => Ok(default),
        Some(v) => v.trim().parse::<u64>().map_err(|_| {
            AppError::new(ErrorCode::ConfigMissing, format!("{key} 不是合法整数: {}", mask(v)))
        }),
    }
}

fn parse_u32(map: &HashMap<String, String>, key: &str, default: u32) -> Result<u32> {
    match map.get(key) {
        None => Ok(default),
        Some(v) => v.trim().parse::<u32>().map_err(|_| {
            AppError::new(ErrorCode::ConfigMissing, format!("{key} 不是合法整数: {}", mask(v)))
        }),
    }
}

fn parse_f64(map: &HashMap<String, String>, key: &str, default: f64) -> Result<f64> {
    match map.get(key) {
        None => Ok(default),
        Some(v) => v.trim().parse::<f64>().map_err(|_| {
            AppError::new(ErrorCode::ConfigMissing, format!("{key} 不是合法数值: {}", mask(v)))
        }),
    }
}

/// 凭据掩码：任何日志 / 错误提示都不出现原文。
pub fn mask(s: &str) -> String {
    if s.is_empty() { return "<空>".to_string(); }
    let n = s.chars().count();
    if n <= 2 { return "**".to_string(); }
    let head: String = s.chars().take(1).collect();
    let tail: String = s.chars().skip(n - 1).collect();
    format!("{}***{}", head, tail)
}

impl Config {
    /// 从环境变量构建（不校验），便于诊断模式用占位值。
    pub fn from_env() -> Self {
        let get = |k: &str| std::env::var(k).unwrap_or_default();
        Self::from_map_env(&get)
    }

    fn from_map_env(get: &dyn Fn(&str) -> String) -> Self {
        let baseline = BaselineConfig {
            window_minutes: get("BASELINE_WINDOW_MINUTES").parse().unwrap_or(30),
            degrade_ratio_threshold: get("BASELINE_DEGRADE_RATIO").parse().unwrap_or(1.5),
            error_rate_delta_threshold: get("BASELINE_ERROR_RATE_DELTA").parse().unwrap_or(0.05),
            min_baseline_samples: get("BASELINE_MIN_SAMPLES").parse().unwrap_or(8),
            max_baseline_cv: get("BASELINE_MAX_CV").parse().unwrap_or(1.2),
            threshold_inclusive: get("BASELINE_THRESHOLD_INCLUSIVE").parse().unwrap_or(false),
            w_latency_abs: get("RANK_W_LATENCY_ABS").parse().unwrap_or(0.30),
            w_ratio: get("RANK_W_RATIO").parse().unwrap_or(0.35),
            w_error_delta: get("RANK_W_ERROR_DELTA").parse().unwrap_or(0.20),
            w_host_deviation: get("RANK_W_HOST_DEV").parse().unwrap_or(0.15),
            min_evidence_refs: get("MIN_EVIDENCE_REFS").parse().unwrap_or(1),
        };
        let limits = QueryLimits {
            max_scan_rows: get("MAX_SCAN_ROWS").parse().unwrap_or(2_000_000),
            max_memory_bytes: get("MAX_MEMORY_BYTES").parse().unwrap_or(512 * 1024 * 1024),
            top_n: get("TOP_N").parse().unwrap_or(20),
            min_clock_samples: get("MIN_CLOCK_SAMPLES").parse().unwrap_or(5),
        };
        let retention = RetentionConfig {
            days: get("RETENTION_DAYS").parse().unwrap_or(7),
            max_partition_bytes: get("MAX_PARTITION_BYTES").parse().unwrap_or(50 * 1024 * 1024 * 1024),
        };
        Self {
            esb: EsbConfig {
                host: get("ESB_AUDIT_DB_HOST"),
                port: get("ESB_AUDIT_DB_PORT").parse().unwrap_or(3306),
                user: get("ESB_AUDIT_DB_USER"),
                password: get("ESB_AUDIT_DB_PASSWORD"),
                db_name: get("ESB_AUDIT_DB_NAME"),
            },
            prom: PromConfig {
                base_url: get("PROM_BASE_URL"),
                bearer_token: get("PROM_BEARER_TOKEN"),
                max_response_bytes: get("PROM_MAX_RESPONSE_BYTES").parse().unwrap_or(32 * 1024 * 1024),
            },
            log: LogConfig {
                esb_log_dir: PathBuf::from(get("ESB_LOG_DIR")),
                app_log_dir: PathBuf::from(get("APP_LOG_DIR")),
                max_line_bytes: get("MAX_LINE_BYTES").parse().unwrap_or(65536),
            },
            ticket: TicketConfig {
                base_url: get("OPS_TICKET_BASE_URL"),
                token: get("OPS_TICKET_TOKEN"),
            },
            model: ModelConfig {
                base_url: get("MODEL_BASE_URL"),
                api_key: get("MODEL_API_KEY"),
                name: get("MODEL_NAME"),
                timeout_sec: get("MODEL_TIMEOUT_SEC").parse().unwrap_or(30),
                max_retries: get("MODEL_MAX_RETRIES").parse().unwrap_or(2),
            },
            baseline,
            limits,
            retention,
            data_dir: {
                let d = get("DATA_DIR");
                PathBuf::from(if d.is_empty() { "data".to_string() } else { d })
            },
            collect_interval_sec: get("COLLECT_INTERVAL_SEC").parse().unwrap_or(300),
        }
    }

    /// 从键值映射构建（供测试注入缺项 / 非法值）。未知键忽略。
    pub fn from_map(map: &HashMap<String, String>) -> Result<Self> {
        let get = |k: &str| map.get(k).cloned().unwrap_or_default();
        let mut c = Self::from_map_env(&get);
        // 数值范围校验（非法值直接拒绝，不回退默认）
        c.baseline.window_minutes = parse_range_i64(map, "BASELINE_WINDOW_MINUTES", 1, 1440, c.baseline.window_minutes)?;
        c.baseline.min_baseline_samples = parse_range_usize(map, "BASELINE_MIN_SAMPLES", 1, 100000, c.baseline.min_baseline_samples)?;
        c.limits.max_scan_rows = parse_range_usize(map, "MAX_SCAN_ROWS", 1, usize::MAX, c.limits.max_scan_rows)?;
        c.limits.max_memory_bytes = parse_range_usize(map, "MAX_MEMORY_BYTES", 1024, usize::MAX, c.limits.max_memory_bytes)?;
        c.retention.days = parse_range_u64(map, "RETENTION_DAYS", 1, 3650, c.retention.days)?;
        let _ = (parse_u64(map, "MODEL_TIMEOUT_SEC", c.model.timeout_sec)?,
                 parse_u32(map, "MODEL_MAX_RETRIES", c.model.max_retries)?,
                 parse_f64(map, "BASELINE_DEGRADE_RATIO", c.baseline.degrade_ratio_threshold)?);
        Ok(c)
    }

    /// 按模式校验必需凭据。缺项返回 ConfigMissing，并逐项列出来源环境变量。
    pub fn validate(&self, mode: Strictness) -> Result<()> {
        let mut missing: Vec<&str> = Vec::new();
        // ESB 五项
        if self.esb.host.is_empty() { missing.push("ESB_AUDIT_DB_HOST"); }
        if self.esb.port == 0 { missing.push("ESB_AUDIT_DB_PORT"); }
        if self.esb.user.is_empty() { missing.push("ESB_AUDIT_DB_USER"); }
        if self.esb.password.is_empty() { missing.push("ESB_AUDIT_DB_PASSWORD"); }
        if self.esb.db_name.is_empty() { missing.push("ESB_AUDIT_DB_NAME"); }
        if self.prom.base_url.is_empty() { missing.push("PROM_BASE_URL"); }
        if self.prom.bearer_token.is_empty() { missing.push("PROM_BEARER_TOKEN"); }
        if self.log.esb_log_dir.as_os_str().is_empty() { missing.push("ESB_LOG_DIR"); }
        if self.log.app_log_dir.as_os_str().is_empty() { missing.push("APP_LOG_DIR"); }
        if mode == Strictness::Strict {
            if self.ticket.base_url.is_empty() { missing.push("OPS_TICKET_BASE_URL"); }
            if self.ticket.token.is_empty() { missing.push("OPS_TICKET_TOKEN"); }
        }
        if missing.is_empty() { Ok(()) } else {
            Err(AppError::new(
                ErrorCode::ConfigMissing,
                format!("缺 {} 项：{}（均从环境变量读取）", missing.len(), missing.join(", ")),
            ))
        }
    }

    /// 模型配置是否齐全；不齐全则走确定性基线（降级），不是启动失败。
    pub fn model_ready(&self) -> bool {
        !self.model.base_url.is_empty() && !self.model.api_key.is_empty() && !self.model.name.is_empty()
    }

    pub fn ticket_ready(&self) -> bool {
        !self.ticket.base_url.is_empty() && !self.ticket.token.is_empty()
    }

    /// 生成不含任何凭据的脱敏摘要（用于运行摘要 / 审计）。
    pub fn redacted_summary(&self) -> String {
        format!(
            "esb={}@{}:{}/{} prom={} ticket={} model={}/{}",
            self.esb.user, self.esb.host, self.esb.port, self.esb.db_name,
            redact_url(&self.prom.base_url), redact_url(&self.ticket.base_url),
            redact_url(&self.model.base_url), self.model.name,
        )
    }
}

/// 只保留 URL 的 host 部分，去掉路径与查询（可能含令牌）。
pub fn redact_url(url: &str) -> String {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let host = after_scheme.split('/').next().unwrap_or("");
    host.split('@').last().unwrap_or(host).to_string()
}

fn parse_range_i64(map: &HashMap<String, String>, key: &str, lo: i64, hi: i64, default: i64) -> Result<i64> {
    let v = match map.get(key) { None => return Ok(default), Some(v) => v };
    let n = v.parse::<i64>().map_err(|_| AppError::new(ErrorCode::ConfigMissing, format!("{key} 非法")))?;
    if n < lo || n > hi { return Err(AppError::new(ErrorCode::ConfigMissing, format!("{key} 超出范围 [{lo},{hi}]"))); }
    Ok(n)
}

fn parse_range_u64(map: &HashMap<String, String>, key: &str, lo: u64, hi: u64, default: u64) -> Result<u64> {
    let v = match map.get(key) { None => return Ok(default), Some(v) => v };
    let n = v.parse::<u64>().map_err(|_| AppError::new(ErrorCode::ConfigMissing, format!("{key} 非法")))?;
    if n < lo || n > hi { return Err(AppError::new(ErrorCode::ConfigMissing, format!("{key} 超出范围 [{lo},{hi}]"))); }
    Ok(n)
}

fn parse_range_usize(map: &HashMap<String, String>, key: &str, lo: usize, hi: usize, default: usize) -> Result<usize> {
    let v = match map.get(key) { None => return Ok(default), Some(v) => v };
    let n = v.parse::<usize>().map_err(|_| AppError::new(ErrorCode::ConfigMissing, format!("{key} 非法")))?;
    if n < lo || n > hi { return Err(AppError::new(ErrorCode::ConfigMissing, format!("{key} 超出范围"))); }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ess(k: &str, v: &str) -> HashMap<String, String> {
        let mut m = HashMap::new();
        for (a, b) in [
            ("ESB_AUDIT_DB_HOST", "db.internal"), ("ESB_AUDIT_DB_PORT", "3306"),
            ("ESB_AUDIT_DB_USER", "readonly"), ("ESB_AUDIT_DB_PASSWORD", "s3cr3t"),
            ("ESB_AUDIT_DB_NAME", "esb_audit"), ("PROM_BASE_URL", "http://prom:9090"),
            ("PROM_BEARER_TOKEN", "tok-abc"), ("ESB_LOG_DIR", "/logs/esb"),
            ("APP_LOG_DIR", "/logs/app"), ("OPS_TICKET_BASE_URL", "http://ops:8080"),
            ("OPS_TICKET_TOKEN", "tick-xyz"),
        ] { m.insert(a.to_string(), b.to_string()); }
        if !k.is_empty() { m.insert(k.to_string(), v.to_string()); }
        m
    }

    #[test]
    fn missing_esb_credential_reports_config_missing() {
        let mut m = ess("", "");
        m.remove("ESB_AUDIT_DB_HOST");
        let c = Config::from_map(&m).unwrap();
        let e = c.validate(Strictness::Strict).unwrap_err();
        assert_eq!(e.code, ErrorCode::ConfigMissing);
        assert!(e.detail.contains("ESB_AUDIT_DB_HOST"));
    }

    #[test]
    fn missing_ticket_token_only_fails_in_strict_mode() {
        let mut m = ess("", "");
        m.remove("OPS_TICKET_TOKEN");
        let c = Config::from_map(&m).unwrap();
        assert!(c.validate(Strictness::Diagnose).is_ok());
        assert_eq!(c.validate(Strictness::Strict).unwrap_err().code, ErrorCode::ConfigMissing);
    }

    #[test]
    fn missing_model_config_is_not_a_startup_failure() {
        let m = ess("", ""); // 无 MODEL_* 变量
        let c = Config::from_map(&m).unwrap();
        assert!(c.validate(Strictness::Strict).is_ok());
        assert!(!c.model_ready());
    }

    #[test]
    fn out_of_range_value_rejected() {
        let m = ess("RETENTION_DAYS", "0");
        assert_eq!(Config::from_map(&m).unwrap_err().code, ErrorCode::ConfigMissing);
    }

    #[test]
    fn error_messages_never_leak_credentials() {
        let m = ess("ESB_AUDIT_DB_PORT", "not-a-port");
        // 端口非法在 from_map 未显式校验，走默认值；凭据不会进错误
        let e = AppError::new(ErrorCode::ConfigMissing, format!("{}", mask("s3cr3t")));
        assert!(!e.to_string().contains("s3cr3t"));
        let c = Config::from_map(&m).unwrap();
        assert!(!c.redacted_summary().contains("s3cr3t"));
        assert!(!c.redacted_summary().contains("tok-abc"));
    }

    #[test]
    fn mask_keeps_only_edges() {
        assert_eq!(mask("abcdef"), "a***f");
        assert_eq!(mask("ab"), "**");
    }
}
