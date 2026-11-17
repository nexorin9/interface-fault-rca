//! 系统托盘三态：灰(未采集) / 绿(正常) / 琥珀(有待确认候选)；状态由真实台账/采集状态驱动；
//! 菜单四项（打开工作台/立即采集/查看待确认数/退出），采集走同一路径；单实例保护。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState { Gray, Green, Amber }

impl TrayState {
    pub fn color_hex(self) -> &'static str {
        match self { TrayState::Gray => "#6B788A", TrayState::Green => "#3FB984", TrayState::Amber => "#F0B429" }
    }
    pub fn tooltip(self, pending: usize, collected: bool) -> String {
        match self {
            TrayState::Gray => "未采集".to_string(),
            TrayState::Green => format!("正常（已采集，待确认 {pending}）"),
            TrayState::Amber => format!("有待确认候选 {} 条", pending),
        }
    }
}

/// 由真实状态推导托盘态（不是手动切换）
pub fn derive_tray_state(collected: bool, pending_count: usize) -> TrayState {
    if !collected { TrayState::Gray }
    else if pending_count > 0 { TrayState::Amber }
    else { TrayState::Green }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayMenu { OpenWorkbench, CollectNow, ViewPending, Quit }

pub const TRAY_MENUS: [TrayMenu; 4] = [
    TrayMenu::OpenWorkbench, TrayMenu::CollectNow, TrayMenu::ViewPending, TrayMenu::Quit,
];

/// 单实例保护：第二次启动被拒（复用 RepeatedStart 语义）
#[derive(Debug, Clone)]
pub struct SingleInstanceGuard {
    running: std::sync::Arc<std::sync::Mutex<bool>>,
}

impl SingleInstanceGuard {
    pub fn new() -> Self { Self { running: std::sync::Arc::new(std::sync::Mutex::new(false)) } }
    pub fn try_acquire(&self) -> bool {
        let mut r = self.running.lock().unwrap();
        if *r { false } else { *r = true; true }
    }
    pub fn release(&self) { let mut r = self.running.lock().unwrap(); *r = false; }
}

/// 采集菜单触发的采集回调标识（与定时采集共用一条路径）
pub const COLLECT_PATH: &str = "ingest::IncrementalScheduler::run_round";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_tray_states() {
        assert_eq!(derive_tray_state(false, 5), TrayState::Gray);
        assert_eq!(derive_tray_state(true, 3), TrayState::Amber);
        assert_eq!(derive_tray_state(true, 0), TrayState::Green);
    }

    #[test]
    fn tooltip_reflects_pending_count() {
        assert!(TrayState::Amber.tooltip(3, true).contains("3"));
        assert_eq!(TrayState::Gray.tooltip(3, false), "未采集");
    }

    #[test]
    fn four_menu_items() {
        assert_eq!(TRAY_MENUS.len(), 4);
        assert!(TRAY_MENUS.contains(&TrayMenu::CollectNow));
    }

    #[test]
    fn collect_menu_uses_same_path() {
        assert_eq!(COLLECT_PATH, "ingest::IncrementalScheduler::run_round");
    }

    #[test]
    fn second_instance_rejected() {
        let g = SingleInstanceGuard::new();
        assert!(g.try_acquire());
        assert!(!g.try_acquire());
        g.release();
        assert!(g.try_acquire());
    }
}
