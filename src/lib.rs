//! 接口故障根因定位 —— 库入口。
//!
//! 桌面程序（main.rs）与全部集成测试共享本库的类型与行为，
//! 保证「诊断命令」「常驻程序」「测试替身」走的是同一条主路径。

pub mod error;
pub mod config;
pub mod observability;
pub mod runtime;
pub mod store;
pub mod ingest;
pub mod binding;
pub mod query;
pub mod baseline;
pub mod ai;
pub mod writeback;
pub mod ui;
pub mod diagnose;
