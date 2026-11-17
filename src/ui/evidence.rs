//! 证据区：三段可折叠（流水/日志/指标），每行带 __row_id 与来源色条，可按 ID 反查原文并定位。

use crate::ui::components::SourceColorBar;
use crate::ui::theme::Rgb;

#[derive(Debug, Clone)]
pub struct EvidenceLine {
    pub row_id: String,
    pub source_kind: String,
    pub ts_ms: i64,
    pub summary: String,
    pub color_bar: SourceColorBar,
}

impl EvidenceLine {
    pub fn new(row_id: &str, source_kind: &str, ts_ms: i64, summary: &str) -> Self {
        Self { row_id: row_id.into(), source_kind: source_kind.into(), ts_ms,
            summary: summary.into(), color_bar: SourceColorBar::new(source_kind) }
    }
    pub fn color(&self) -> Rgb { self.color_bar.color }
}

#[derive(Debug, Clone)]
pub struct EvidenceSection {
    pub kind: String,
    pub collapsed: bool,
    pub lines: Vec<EvidenceLine>,
}

#[derive(Debug, Clone)]
pub struct EvidencePanel {
    pub sections: Vec<EvidenceSection>, // flow / log / metric 三段
}

impl EvidencePanel {
    /// 从带 row_id 的证据行装配三段；每段可折叠。
    pub fn from_lines(lines: Vec<EvidenceLine>) -> Self {
        let mut sections = Vec::new();
        for kind in ["flow", "log", "metric"] {
            let part: Vec<_> = lines.iter().filter(|l| l.source_kind == kind).cloned().collect();
            sections.push(EvidenceSection { kind: kind.into(), collapsed: false, lines: part });
        }
        Self { sections }
    }

    pub fn three_sections(&self) -> bool {
        self.sections.len() == 3 && self.sections.iter().all(|s| ["flow", "log", "metric"].contains(&s.kind.as_str()))
    }

    pub fn every_line_has_row_id(&self) -> bool {
        self.sections.iter().flat_map(|s| s.lines.iter()).all(|l| !l.row_id.is_empty())
    }

    /// 按 __row_id 定位（反查命中）
    pub fn locate(&self, row_id: &str) -> Option<&EvidenceLine> {
        self.sections.iter().flat_map(|s| s.lines.iter()).find(|l| l.row_id == row_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel() -> EvidencePanel {
        EvidencePanel::from_lines(vec![
            EvidenceLine::new("flow#a1", "flow", 100, "rc=200 dur=300ms"),
            EvidenceLine::new("log#a2", "log", 110, "ERROR timeout"),
            EvidenceLine::new("metric#a3", "metric", 120, "p95=300"),
        ])
    }

    #[test]
    fn three_foldable_sections_exist() {
        assert!(panel().three_sections());
    }

    #[test]
    fn every_line_carries_row_id_and_source_color() {
        let p = panel();
        assert!(p.every_line_has_row_id());
        for s in &p.sections {
            for l in &s.lines { assert_ne!(l.color().hex().len(), 0); }
        }
    }

    #[test]
    fn locate_by_row_id() {
        let p = panel();
        assert!(p.locate("flow#a1").is_some());
        assert!(p.locate("flow#fake").is_none());
    }

    #[test]
    fn reference_can_jump_to_evidence_line() {
        let p = panel();
        let line = p.locate("log#a2").unwrap();
        assert_eq!(line.source_kind, "log");
        assert!(line.summary.contains("timeout"));
    }
}
