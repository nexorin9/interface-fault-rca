//! M1 摄取管线与来源适配器。

pub mod pipeline;
pub mod dispatcher;
pub mod schema_map;
pub mod esb_mysql;
pub mod prom_http;
pub mod log_tail;
pub mod contract;
pub mod scheduler;
pub mod jsonl;

pub use pipeline::*;
pub use dispatcher::*;
pub use schema_map::*;
pub use contract::*;
