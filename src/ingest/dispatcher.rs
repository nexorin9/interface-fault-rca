//! 按 source 路由到 event_kind（esb_audit→flow / app_log→log / prom→metric）。
//! 路由规则可配置——改配置即改路由，不改代码。

use crate::error::{AppError, ErrorCode, Result};
use std::collections::HashMap;

pub fn default_routing() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("esb_audit".into(), "flow".into());
    m.insert("app_log".into(), "log".into());
    m.insert("prom".into(), "metric".into());
    m
}

#[derive(Debug, Clone)]
pub struct Dispatcher {
    routing: HashMap<String, String>,
}

impl Default for Dispatcher {
    fn default() -> Self { Self { routing: default_routing() } }
}

impl Dispatcher {
    pub fn new(routing: HashMap<String, String>) -> Self { Self { routing } }

    pub fn route(&self, source: &str) -> Result<String> {
        self.routing.get(source).cloned().ok_or_else(|| {
            AppError::new(ErrorCode::SchemaMismatch, format!("来源 {source} 无 event_kind 路由规则"))
        })
    }

    pub fn set_route(&mut self, source: &str, kind: &str) {
        self.routing.insert(source.to_string(), kind.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_routing_matches_spec() {
        let d = Dispatcher::default();
        assert_eq!(d.route("esb_audit").unwrap(), "flow");
        assert_eq!(d.route("app_log").unwrap(), "log");
        assert_eq!(d.route("prom").unwrap(), "metric");
    }

    #[test]
    fn configurable_route_no_code_change() {
        let mut d = Dispatcher::default();
        d.set_route("prom", "custom_metric");
        assert_eq!(d.route("prom").unwrap(), "custom_metric");
    }

    #[test]
    fn unknown_source_is_error() {
        assert_eq!(Dispatcher::default().route("unknown").unwrap_err().code, ErrorCode::SchemaMismatch);
    }
}
