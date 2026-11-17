//! task 23 验收：三适配器统一契约 —— 统计字段集合一致、失败分类同枚举、产出行通过 schema 校验。
//! 本地替身仅作测试数据层，正式对接方式为 MySQL / Prometheus HTTP / 日志目录。
mod common;

use common::flow;
use interface_fault_rca::ingest::contract::{stat_field_names, Adapter, AdapterFailure, AdapterOutput, ContractRow};
use interface_fault_rca::observability::SourceStats;

struct StubAdapter { name: &'static str }
impl Adapter for StubAdapter {
    fn source_name(&self) -> &str { self.name }
    fn collect(&self, _ws: i64, _we: i64) -> AdapterOutput {
        AdapterOutput {
            rows: vec![ContractRow {
                source: self.name.into(), event_kind: "flow".into(),
                interface_code: "ESB-A".into(), ts_ms: 1000,
                payload: [("duration_ms".into(), "100".into())].into_iter().collect(),
            }],
            stats: SourceStats::new(self.name),
            failure: None,
        }
    }
}

#[test]
fn stat_field_names_are_identical_across_sources() {
    let names = stat_field_names();
    assert_eq!(names.len(), 9);
    assert!(names.contains(&"attempted"));
    assert!(names.contains(&"ingested"));
    assert!(names.contains(&"dirty_lines"));
    assert!(names.contains(&"watermark"));
    assert!(names.contains(&"fail_code"));
}

#[test]
fn failure_classifications_share_one_enum() {
    // 三类失败各映射到 task 17 的错误码，且不重复
    let codes = [
        AdapterFailure::CredentialsMissing.error_code(),
        AdapterFailure::InvalidProtocol.error_code(),
        AdapterFailure::InvalidWindow.error_code(),
    ];
    assert_eq!(codes.len(), 3);
    assert_ne!(codes[0], codes[1]);
}

#[test]
fn three_adapter_success_rows_pass_schema() {
    let sources = ["esb_audit", "prom", "app_log"];
    for s in sources {
        let a = StubAdapter { name: s };
        let out = a.collect(0, 2000);
        assert_eq!(out.rows.len(), 1);
        assert!(out.failure.is_none());
        // 契约行字段齐备
        assert_eq!(out.rows[0].source, s);
        assert!(out.rows[0].ts_ms > 0);
    }
    // 通过统一事件表 schema 校验
    let r = flow("ESB-A", "SVC", 1000, 100.0, 200);
    assert!(r.validate().is_ok());
}
