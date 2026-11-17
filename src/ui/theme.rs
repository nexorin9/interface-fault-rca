//! 设计 token：深色运维工作台（主背景 #14181F）、四语义色、8px 栅格、字体层级、四态。
//! 纪律（对齐 uis/UI_BASELINE）：令牌先行（源码不硬编码色值）、来源→语义色映射唯一且单测锁定、
//! 四语义色在 #14181F 上对比度 ≥ 3:1、不用红绿对立、不用 emoji 当图标。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb { pub r: u8, pub g: u8, pub b: u8 }

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self { Self { r, g, b } }
    pub fn hex(self) -> String { format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b) }
    /// 相对亮度（WCAG）
    pub fn luminance(self) -> f64 {
        let f = |v: u8| {
            let c = v as f64 / 255.0;
            if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * f(self.r) + 0.7152 * f(self.g) + 0.0722 * f(self.b)
    }
}

/// WCAG 对比度
pub fn contrast(a: Rgb, b: Rgb) -> f64 {
    let l1 = a.luminance().max(b.luminance());
    let l2 = a.luminance().min(b.luminance());
    (l1 + 0.05) / (l2 + 0.05)
}

// ---- 色板 token（全部集中于此，业务代码只引用常量）----
pub const BG_MAIN: Rgb = Rgb::new(0x14, 0x18, 0x1F);
pub const BG_PANEL: Rgb = Rgb::new(0x1C, 0x22, 0x2C);
pub const BG_ELEVATED: Rgb = Rgb::new(0x24, 0x2C, 0x38);
pub const TEXT_PRIMARY: Rgb = Rgb::new(0xE6, 0xED, 0xF5);
pub const TEXT_SECONDARY: Rgb = Rgb::new(0x9A, 0xA8, 0xB8);
pub const TEXT_MUTED: Rgb = Rgb::new(0x6B, 0x78, 0x8A);
pub const BORDER: Rgb = Rgb::new(0x30, 0x3A, 0x48);

// 四语义色（不用红绿对立）
pub const FLOW_BLUE: Rgb = Rgb::new(0x4C, 0x8D, 0xFF);
pub const LOG_ORANGE: Rgb = Rgb::new(0xE8, 0xA3, 0x3D);
pub const METRIC_GREEN: Rgb = Rgb::new(0x3F, 0xB9, 0x84);
pub const PENDING_AMBER: Rgb = Rgb::new(0xF0, 0xB4, 0x29);

pub const SEMANTIC_COLORS: [(&str, Rgb); 4] = [
    ("flow", FLOW_BLUE), ("log", LOG_ORANGE), ("metric", METRIC_GREEN), ("pending", PENDING_AMBER),
];

/// 来源 → 语义色映射（常量表，唯一入口）
pub fn source_color(source_kind: &str) -> Option<Rgb> {
    SEMANTIC_COLORS.iter().find(|(k, _)| *k == source_kind).map(|(_, c)| *c)
}

pub fn pending_color() -> Rgb { PENDING_AMBER }

// ---- 8px 栅格 ----
pub const SPACE_1: f32 = 4.0;
pub const SPACE_2: f32 = 8.0;
pub const SPACE_3: f32 = 12.0;
pub const SPACE_4: f32 = 16.0;
pub const SPACE_6: f32 = 24.0;
pub const SPACE_8: f32 = 32.0;

// ---- 字体层级（等宽用于接口编码与延迟数值）----
pub const FONT_MONO: &str = "ui-monospace, SF Mono, Menlo, Consolas, monospace";
pub const FONT_SANS: &str = "-apple-system, PingFang SC, Microsoft YaHei, sans-serif";
pub const TEXT_XS: f32 = 12.0;
pub const TEXT_SM: f32 = 13.0;
pub const TEXT_BASE: f32 = 14.0;
pub const TEXT_LG: f32 = 16.0;
pub const TEXT_XL: f32 = 20.0;

// ---- 四态 token ----
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetState { Hover, Active, Disabled, Loading }

pub fn state_alpha(s: WidgetState) -> f32 {
    match s { WidgetState::Hover => 0.08, WidgetState::Active => 0.16, WidgetState::Disabled => 0.04, WidgetState::Loading => 0.10 }
}

/// 时长 token（动效）：120/180/240/320ms
pub const DUR_1_MS: u64 = 120;
pub const DUR_2_MS: u64 = 180;
pub const DUR_3_MS: u64 = 240;
pub const DUR_4_MS: u64 = 320;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_color_mapping_is_unique() {
        // 一个来源 kind 只对应一个语义色（映射表键唯一）
        let keys: std::collections::BTreeSet<_> = SEMANTIC_COLORS.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys.len(), 4);
        assert_eq!(source_color("flow"), Some(FLOW_BLUE));
        assert_eq!(source_color("log"), Some(LOG_ORANGE));
        assert_eq!(source_color("metric"), Some(METRIC_GREEN));
        assert_eq!(source_color("不存在"), None);
    }

    #[test]
    fn semantic_colors_contrast_against_bg_at_least_3_to_1() {
        for (name, c) in SEMANTIC_COLORS {
            let cr = contrast(c, BG_MAIN);
            assert!(cr >= 3.0, "语义色 {name} 对比度 {cr:.2} < 3:1");
        }
    }

    #[test]
    fn no_duplicate_token_definitions() {
        // 四语义色两两不同
        for i in 0..SEMANTIC_COLORS.len() {
            for j in (i + 1)..SEMANTIC_COLORS.len() {
                assert_ne!(SEMANTIC_COLORS[i].1, SEMANTIC_COLORS[j].1);
            }
        }
    }

    #[test]
    fn hex_format_correct() {
        assert_eq!(BG_MAIN.hex(), "#14181F");
        assert_eq!(FLOW_BLUE.hex(), "#4C8DFF");
    }

    #[test]
    fn grid_is_multiple_of_eight_except_hairline() {
        for v in [SPACE_2, SPACE_4, SPACE_6, SPACE_8] { assert_eq!(v % 8.0, 0.0); }
    }
}
