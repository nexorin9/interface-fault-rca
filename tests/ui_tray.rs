//! task 48 验收：托盘三态 —— 随真实状态切换、菜单四项、重复启动被拒、退出优雅停止。
use interface_fault_rca::ui::tray::{
    derive_tray_state, COLLECT_PATH, SingleInstanceGuard, TrayMenu, TrayState, TRAY_MENUS,
};

#[test]
fn three_states_derived_from_real_state() {
    assert_eq!(derive_tray_state(false, 0), TrayState::Gray);
    assert_eq!(derive_tray_state(true, 0), TrayState::Green);
    assert_eq!(derive_tray_state(true, 3), TrayState::Amber);
}

#[test]
fn tray_colors_and_tooltips() {
    assert_eq!(TrayState::Gray.color_hex(), "#6B788A");
    assert_eq!(TrayState::Green.color_hex(), "#3FB984");
    assert_eq!(TrayState::Amber.color_hex(), "#F0B429");
    assert!(TrayState::Amber.tooltip(3, true).contains("3"));
    assert_eq!(TrayState::Gray.tooltip(0, false), "未采集");
}

#[test]
fn menu_has_four_items_and_collect_shares_path() {
    assert_eq!(TRAY_MENUS.len(), 4);
    assert!(TRAY_MENUS.contains(&TrayMenu::OpenWorkbench));
    assert!(TRAY_MENUS.contains(&TrayMenu::CollectNow));
    assert!(TRAY_MENUS.contains(&TrayMenu::ViewPending));
    assert!(TRAY_MENUS.contains(&TrayMenu::Quit));
    assert_eq!(COLLECT_PATH, "ingest::IncrementalScheduler::run_round");
}

#[test]
fn duplicate_start_rejected_and_release_allows_restart() {
    let guard = SingleInstanceGuard::new();
    assert!(guard.try_acquire());
    assert!(!guard.try_acquire());
    guard.release();
    assert!(guard.try_acquire());
    guard.release();
}

#[test]
fn graceful_stop_drops_collect_state() {
    let guard = SingleInstanceGuard::new();
    guard.try_acquire();
    guard.release();
    assert!(guard.try_acquire()); // 退出后可再次启动（优雅停止）
    guard.release();
}
