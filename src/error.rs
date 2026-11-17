//! 语义化错误码（对应 plan 中「错误按语义分类穷举」纪律）。
//! 禁止用裸字符串表达这些情形；每个码带面向用户的中文提示与建议动作。

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    SourceUnavailable,
    SchemaMismatch,
    AmbiguousInterface,
    EvidenceMissing,
    ModelUnavailable,
    WindowTooLarge,
    ConfigMissing,
    RepeatedStart,
    OutboundBlocked,
    TicketRejected,
}

impl ErrorCode {
    /// 面向用户的中文提示
    pub fn message(self) -> &'static str {
        match self {
            ErrorCode::SourceUnavailable => "数据来源不可用",
            ErrorCode::SchemaMismatch => "来源结构不符合映射表",
            ErrorCode::AmbiguousInterface => "接口码映射存在歧义",
            ErrorCode::EvidenceMissing => "证据引用缺失或非法",
            ErrorCode::ModelUnavailable => "模型服务不可用",
            ErrorCode::WindowTooLarge => "查询窗口过大，已降级",
            ErrorCode::ConfigMissing => "缺少必需配置或凭据",
            ErrorCode::RepeatedStart => "采集任务重复启动",
            ErrorCode::OutboundBlocked => "未确认前禁止出站",
            ErrorCode::TicketRejected => "工单系统拒绝建单",
        }
    }

    /// 建议值班人采取的动作
    pub fn suggestion(self) -> &'static str {
        match self {
            ErrorCode::SourceUnavailable => "检查对应来源的地址、端口与只读凭据，或稍后重试。",
            ErrorCode::SchemaMismatch => "核对来源实际列名与 config/pipelines.toml 映射表。",
            ErrorCode::AmbiguousInterface => "到 ESB 接口登记表核对一码多服务，补全字典后重跑。",
            ErrorCode::EvidenceMissing => "检查证据行 ID 是否落在本窗口白名单内。",
            ErrorCode::ModelUnavailable => "已自动切换到规则基线结论，可人工补原因后建单。",
            ErrorCode::WindowTooLarge => "缩小时间窗，或调高扫描上限；当前仅返回 Top-N。",
            ErrorCode::ConfigMissing => "按 .env.example 补齐缺失的环境变量后重启。",
            ErrorCode::RepeatedStart => "已有采集实例在运行，不要启动第二份。",
            ErrorCode::OutboundBlocked => "值班人先在建单弹窗确认，程序不会在确认前发出任何出站请求。",
            ErrorCode::TicketRejected => "查看工单系统返回的状态码与错误摘要，勿重复提交。",
        }
    }

    /// 稳定字符串标识（用于测试与结构化日志的 code 字段）
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::SourceUnavailable => "SourceUnavailable",
            ErrorCode::SchemaMismatch => "SchemaMismatch",
            ErrorCode::AmbiguousInterface => "AmbiguousInterface",
            ErrorCode::EvidenceMissing => "EvidenceMissing",
            ErrorCode::ModelUnavailable => "ModelUnavailable",
            ErrorCode::WindowTooLarge => "WindowTooLarge",
            ErrorCode::ConfigMissing => "ConfigMissing",
            ErrorCode::RepeatedStart => "RepeatedStart",
            ErrorCode::OutboundBlocked => "OutboundBlocked",
            ErrorCode::TicketRejected => "TicketRejected",
        }
    }
}

/// 应用错误：错误码 + 细节（细节中不得含凭据或日志正文）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppError {
    pub code: ErrorCode,
    pub detail: String,
}

impl AppError {
    pub fn new(code: ErrorCode, detail: impl Into<String>) -> Self {
        Self { code, detail: detail.into() }
    }

    pub fn code_only(code: ErrorCode) -> Self {
        Self { code, detail: String::new() }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.detail.is_empty() {
            write!(f, "[{}] {}。{}", self.code.as_str(), self.code.message(), self.code.suggestion())
        } else {
            write!(f, "[{}] {}：{}。{}", self.code.as_str(), self.code.message(), self.detail, self.code.suggestion())
        }
    }
}

impl std::error::Error for AppError {}

pub type Result<T> = std::result::Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_ten_codes_have_distinct_names_and_messages() {
        let codes = [
            ErrorCode::SourceUnavailable, ErrorCode::SchemaMismatch,
            ErrorCode::AmbiguousInterface, ErrorCode::EvidenceMissing,
            ErrorCode::ModelUnavailable, ErrorCode::WindowTooLarge,
            ErrorCode::ConfigMissing, ErrorCode::RepeatedStart,
            ErrorCode::OutboundBlocked, ErrorCode::TicketRejected,
        ];
        assert_eq!(codes.len(), 10);
        for c in codes {
            assert!(!c.as_str().is_empty());
            assert!(!c.message().is_empty());
            assert!(!c.suggestion().is_empty());
        }
        let names: std::collections::BTreeSet<_> = codes.iter().map(|c| c.as_str()).collect();
        assert_eq!(names.len(), 10);
    }

    #[test]
    fn display_includes_code_message_and_suggestion() {
        let e = AppError::new(ErrorCode::ConfigMissing, "缺 ESB_AUDIT_DB_HOST");
        let s = e.to_string();
        assert!(s.contains("ConfigMissing"));
        assert!(s.contains("缺 ESB_AUDIT_DB_HOST"));
        assert!(s.contains("环境变量"));
    }
}
