//! 三适配器统一契约：输入(凭据+窗口+映射) → 输出(事件行 + 采集统计 + 失败分类)。
//! 三源实现同一契约，采集统计字段集合一致，失败分类取值都在同一枚举内。

use crate::error::ErrorCode;
use crate::observability::SourceStats;
use std::collections::BTreeMap;

/// 统一失败分类（与 task17 语义错误码一一对应）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterFailure {
    CredentialsMissing,   // SourceUnavailable
    InvalidProtocol,      // SchemaMismatch / SourceUnavailable
    InvalidWindow,        // WindowTooLarge / ConfigMissing
}

impl AdapterFailure {
    pub fn error_code(self) -> ErrorCode {
        match self {
            AdapterFailure::CredentialsMissing => ErrorCode::SourceUnavailable,
            AdapterFailure::InvalidProtocol => ErrorCode::SchemaMismatch,
            AdapterFailure::InvalidWindow => ErrorCode::WindowTooLarge,
        }
    }
}

/// 统一契约输出
#[derive(Debug, Clone)]
pub struct AdapterOutput {
    pub rows: Vec<ContractRow>,
    pub stats: SourceStats,
    pub failure: Option<AdapterFailure>,
}

/// 契约行：符合 spec §3.1 字段规范的最小规范形态
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractRow {
    pub source: String,
    pub event_kind: String,
    pub interface_code: String,
    pub ts_ms: i64,
    pub payload: BTreeMap<String, String>,
}

/// 契约 trait：三源实现同一入口形态（这里以函数签名表达，便于测试替身实现）
pub trait Adapter {
    fn source_name(&self) -> &str;
    fn collect(&self, window_start: i64, window_end: i64) -> AdapterOutput;
}

/// 统计字段集合（口径固定）= SourceStats 的字段名；跨源一致性断言据此。
pub fn stat_field_names() -> Vec<&'static str> {
    vec!["source", "attempted", "ingested", "dirty_lines", "too_long", "skipped", "elapsed_ms", "watermark", "fail_code"]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone)]
    struct FakeAdapter { name: String }
    impl Adapter for FakeAdapter {
        fn source_name(&self) -> &str { &self.name }
        fn collect(&self, _a: i64, _b: i64) -> AdapterOutput {
            AdapterOutput { rows: vec![], stats: SourceStats::new(&self.name), failure: None }
        }
    }

    #[test]
    fn three_sources_share_same_stat_fields() {
        let names = stat_field_names();
        assert!(names.contains(&"dirty_lines") && names.contains(&"watermark"));
        let a = SourceStats::new("esb_audit");
        let b = SourceStats::new("prom");
        assert_eq!(a.dirty_lines, b.dirty_lines); // 同结构体同口径
    }

    #[test]
    fn failure_classification_maps_to_error_codes() {
        assert_eq!(AdapterFailure::CredentialsMissing.error_code(), ErrorCode::SourceUnavailable);
        assert_eq!(AdapterFailure::InvalidProtocol.error_code(), ErrorCode::SchemaMismatch);
        assert_eq!(AdapterFailure::InvalidWindow.error_code(), ErrorCode::WindowTooLarge);
    }

    #[test]
    fn contract_row_carries_spec_fields() {
        let mut payload = BTreeMap::new();
        payload.insert("duration_ms".into(), "100".into());
        let row = ContractRow {
            source: "esb_audit".into(), event_kind: "flow".into(),
            interface_code: "A".into(), ts_ms: 1, payload,
        };
        assert_eq!(row.payload.get("duration_ms").unwrap(), "100");
    }

    #[test]
    fn fake_adapter_implements_contract() {
        let a = FakeAdapter { name: "app_log".into() };
        assert_eq!(a.source_name(), "app_log");
        assert!(a.collect(0, 10).failure.is_none());
    }
}
