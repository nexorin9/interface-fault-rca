//! 版本化摄取管线：PipelineDefinition{name,version} + parse→transform→dispatch 三段；
//! 按 (name,version) 精确取用（取不到报错，不回退任意版本）；版本号进入每行审计元数据。

use crate::binding::interface_dict::{Binding, Dictionary};
use crate::error::{AppError, ErrorCode, Result};
use crate::store::event_schema::EventRow;
use crate::store::tsid::{compute_tsid, RowIdAllocator};
use std::collections::BTreeMap;
use std::sync::Arc;

/// 来源原始记录（适配器统一产出形态）
#[derive(Debug, Clone)]
pub struct RawRecord {
    pub source: String,
    /// 来源字段（来源列名 → 原值字符串）
    pub columns: BTreeMap<String, String>,
    /// 已归一到毫秒的时间戳（适配器负责归一）
    pub ts_ms: i64,
}

#[derive(Debug, Clone)]
pub struct PipelineDefinition {
    pub name: String,
    pub version: String,
    pub source: String,
    pub event_kind: String,
}

#[derive(Debug, Clone)]
pub struct PipelineOutcome {
    pub rows: Vec<EventRow>,
    /// 被字典拒绝（歧义/unmapped）的候选——不进事件表候选集
    pub rejected: Vec<String>,
    pub dirty_lines: usize,
}

/// 管线注册表：按 (name, version) 精确检索
#[derive(Debug, Clone, Default)]
pub struct PipelineRegistry {
    definitions: Vec<PipelineDefinition>,
}

impl PipelineRegistry {
    pub fn new() -> Self { Self::default() }

    pub fn register(&mut self, def: PipelineDefinition) {
        self.definitions.push(def);
    }

    pub fn get(&self, name: &str, version: &str) -> Result<PipelineDefinition> {
        self.definitions.iter()
            .find(|d| d.name == name && d.version == version)
            .cloned()
            .ok_or_else(|| AppError::new(ErrorCode::ConfigMissing,
                format!("管线 {name}@{version} 不存在（按名+版本精确取用）")))
    }

    pub fn versions_of(&self, name: &str) -> Vec<String> {
        self.definitions.iter().filter(|d| d.name == name).map(|d| d.version.clone()).collect()
    }
}

/// 管线执行器（按某版本定义执行三段）
pub struct Pipeline {
    pub def: PipelineDefinition,
    dict: Arc<Dictionary>,
    allocator: Arc<RowIdAllocator>,
}

impl Pipeline {
    pub fn new(def: PipelineDefinition, dict: Arc<Dictionary>, allocator: Arc<RowIdAllocator>) -> Self {
        Self { def, dict, allocator }
    }

    /// parse：RawRecord 的 columns 已是键值；这里做类型/脏行判定入口
    fn parse(&self, raw: &RawRecord) -> std::result::Result<(), String> {
        if raw.columns.is_empty() { return Err("空记录".to_string()); }
        Ok(())
    }

    /// transform：列 → tags/fields，绑定字典归一，生成 row_id / tsid，写 pipeline_version 审计元数据
    fn transform(&self, raw: &RawRecord) -> Result<EventRow> {
        let raw_code = raw.columns.get("interface_code").cloned().unwrap_or_default();
        let binding = self.dict.resolve(&raw_code)?;
        let (code, service, unbound) = match binding {
            Binding::Bound { code, service, .. } => (code, service, "false".to_string()),
            Binding::Unbound { raw } => (raw.clone(), String::new(), "true".to_string()),
        };
        let mut tags = BTreeMap::new();
        tags.insert("source".into(), raw.source.clone());
        tags.insert("event_kind".into(), self.def.event_kind.clone());
        tags.insert("interface_code".into(), code.clone());
        if !service.is_empty() { tags.insert("service_name".into(), service); }
        let mut fields = BTreeMap::new();
        for (k, v) in &raw.columns {
            let allowed = match self.def.event_kind.as_str() {
                "flow" => ["response_code", "duration_ms", "caller_system", "raw_value", "pipeline_version", "unbound"].as_slice(),
                "log" => ["level", "message", "stack_head", "raw_value", "pipeline_version", "unbound"].as_slice(),
                "metric" => ["metric_name", "value", "labels", "raw_value", "pipeline_version", "host_deviation", "clock_shift_ms", "unbound"].as_slice(),
                _ => &[],
            };
            if allowed.contains(&k.as_str()) { fields.insert(k.clone(), v.clone()); }
        }
        fields.insert("pipeline_version".into(), format!("{}@{}", self.def.name, self.def.version));
        fields.insert("unbound".into(), unbound);
        let row_id = self.allocator.next(&raw.source);
        let tsid = compute_tsid(&raw.source, raw.ts_ms, &code,
            raw.columns.get("service_name").map(|s| s.as_str()).unwrap_or(""), &self.def.event_kind);
        EventRow::new(tags, raw.ts_ms, fields, row_id, tsid)
    }

    /// dispatch：按 event_kind 分流（本库已统一事件表，这里返回带正确 event_kind 的行）
    fn dispatch(&self, row: EventRow) -> Result<EventRow> {
        if row.kind() != self.def.event_kind {
            return Err(AppError::new(ErrorCode::SchemaMismatch, "dispatch 路由不一致"));
        }
        Ok(row)
    }

    /// 执行：parse → transform → dispatch。歧义/unmapped 按策略处理；脏行计数。
    pub fn execute(&self, records: &[RawRecord]) -> Result<PipelineOutcome> {
        let mut rows = vec![];
        let mut rejected = vec![];
        let mut dirty = 0usize;
        for r in records {
            if self.parse(r).is_err() { dirty += 1; continue; }
            match self.transform(r).and_then(|x| self.dispatch(x)) {
                Ok(row) => rows.push(row),
                Err(e) => {
                    // 歧义/unmapped 不进候选；记录原因（unmapped 其实保留为 unbound 行，不在此分支）
                    rejected.push(format!("{}: {}", r.columns.get("interface_code").map(|s| s.as_str()).unwrap_or("?"), e.code.as_str()));
                }
            }
        }
        Ok(PipelineOutcome { rows, rejected, dirty_lines: dirty })
    }
}

/// 从 config/pipelines.toml 文本构建默认注册表（两条版本 v1/v2 供版本化测试）
pub fn build_registry_from_toml(txt: &str) -> Result<PipelineRegistry> {
    let v: toml::Value = toml::from_str(txt)
        .map_err(|e| AppError::new(ErrorCode::ConfigMissing, format!("管线配置解析失败: {e}")))?;
    let mut reg = PipelineRegistry::new();
    if let Some(arr) = v.get("pipeline").and_then(|x| x.as_array()) {
        for p in arr {
            reg.register(PipelineDefinition {
                name: p.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                version: p.get("version").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                source: p.get("source").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                event_kind: p.get("event_kind").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            });
        }
    }
    Ok(reg)
}

pub fn default_registry() -> PipelineRegistry {
    let mut r = PipelineRegistry::new();
    for (name, source, kind) in [
        ("esb_audit", "esb_audit", "flow"),
        ("app_log", "app_log", "log"),
        ("prom", "prom", "metric"),
    ] {
        r.register(PipelineDefinition { name: name.into(), version: "v1".into(), source: source.into(), event_kind: kind.into() });
        r.register(PipelineDefinition { name: name.into(), version: "v2".into(), source: source.into(), event_kind: kind.into() });
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binding::interface_dict::minimal_dict;

    fn rec(code: &str) -> RawRecord {
        let mut c = BTreeMap::new();
        c.insert("interface_code".into(), code.into());
        c.insert("response_code".into(), "200".into());
        RawRecord { source: "esb_audit".into(), columns: c, ts_ms: 1000 }
    }

    #[test]
    fn pipeline_version_in_audit_metadata() {
        let def = default_registry().get("esb_audit", "v1").unwrap();
        let p = Pipeline::new(def, Arc::new(minimal_dict()), Arc::new(RowIdAllocator::new()));
        let out = p.execute(&[rec("ESB-ORDER-CREATE")]).unwrap();
        assert_eq!(out.rows.len(), 1);
        assert_eq!(out.rows[0].fields.get("pipeline_version").unwrap(), "esb_audit@v1");
    }

    #[test]
    fn get_nonexistent_version_errors() {
        assert_eq!(default_registry().get("esb_audit", "v9").unwrap_err().code, ErrorCode::ConfigMissing);
    }

    #[test]
    fn v1_v2_are_independently_retrievable() {
        let r = default_registry();
        assert!(r.versions_of("esb_audit").contains(&"v1".to_string()));
        assert!(r.versions_of("esb_audit").contains(&"v2".to_string()));
    }

    #[test]
    fn unbound_rows_are_kept_but_marked() {
        let def = default_registry().get("esb_audit", "v1").unwrap();
        let p = Pipeline::new(def, Arc::new(minimal_dict()), Arc::new(RowIdAllocator::new()));
        let out = p.execute(&[rec("UNKNOWN")]).unwrap();
        assert_eq!(out.rows.len(), 1);
        assert_eq!(out.rows[0].fields.get("unbound").unwrap(), "true");
    }
}
