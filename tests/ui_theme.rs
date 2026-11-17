//! task 12 验收：设计 token —— 来源→语义色映射唯一、四语义色在 #14181F 上对比度 ≥3:1、token 无重复。
use interface_fault_rca::ui::theme::{
    contrast, source_color, BG_MAIN, FLOW_BLUE, LOG_ORANGE, METRIC_GREEN, PENDING_AMBER, SEMANTIC_COLORS,
};

#[test]
fn source_to_semantic_color_mapping_is_unique() {
    assert_eq!(source_color("flow"), Some(FLOW_BLUE));
    assert_eq!(source_color("log"), Some(LOG_ORANGE));
    assert_eq!(source_color("metric"), Some(METRIC_GREEN));
    assert_eq!(source_color("pending"), Some(PENDING_AMBER));
    assert_eq!(source_color("unknown"), None);
    let keys: std::collections::BTreeSet<_> = SEMANTIC_COLORS.iter().map(|(k, _)| *k).collect();
    assert_eq!(keys.len(), 4);
}

#[test]
fn four_semantic_colors_contrast_at_least_3_to_1() {
    for (name, c) in SEMANTIC_COLORS {
        let cr = contrast(c, BG_MAIN);
        assert!(cr >= 3.0, "{name} 对比度 {cr:.2} < 3:1");
    }
}

#[test]
fn semantic_colors_are_distinct() {
    let colors = [FLOW_BLUE, LOG_ORANGE, METRIC_GREEN, PENDING_AMBER];
    for i in 0..colors.len() {
        for j in (i + 1)..colors.len() {
            assert_ne!(colors[i], colors[j]);
        }
    }
}

#[test]
fn main_background_is_dark_workbench() {
    assert_eq!(BG_MAIN.hex(), "#14181F");
}
