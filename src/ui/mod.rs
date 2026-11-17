//! M7 桌面界面：设计 token、基础组件、主工作台、证据区、归因卡、确认弹窗、托盘。

pub mod theme;
pub mod components;
pub mod workbench;
pub mod evidence;
pub mod attribution_card;
pub mod confirm;
pub mod states;
pub mod tray;

pub use theme::*;
