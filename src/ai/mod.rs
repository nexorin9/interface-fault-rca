//! M6 模型网关与归因编排（核心差异化模块）。

pub mod conclusion;
pub mod ports;
pub mod evidence_gate;
pub mod attribution;
pub mod schema;
pub mod prompt;
pub mod errors;
pub mod gateway;
pub mod audit_meta;

pub use conclusion::*;
pub use ports::*;
pub use evidence_gate::*;
pub use attribution::*;
pub use schema::*;
