//! 接口故障根因定位 —— 桌面常驻程序入口（主窗 + 托盘）+ 运维诊断子命令。
//! diagnose / report / clean 三条命令复用同一摄取/归因核心，不依赖 GUI 与模型可达。
//!
//! 判定主路径在库里（`interface_fault_rca::diagnose::diagnose`），
//! 本文件只做「参数解析 → 调主路径 → 人可读回显 → 落产物」，不另写一套判定逻辑。

use interface_fault_rca::ai::attribution::record_candidates;
use interface_fault_rca::ai::gateway::ModelGatewayClient;
use interface_fault_rca::ai::ports::ModelGateway;
use interface_fault_rca::binding::interface_dict::{minimal_dict, Dictionary};
use interface_fault_rca::config::{Config, Strictness};
use interface_fault_rca::diagnose::{diagnose, DiagnoseOptions};
use interface_fault_rca::error::{AppError, ErrorCode, Result};
use interface_fault_rca::ingest::jsonl;
use interface_fault_rca::ingest::pipeline::RawRecord;
use interface_fault_rca::store::sqlite_index::IndexDb;
use interface_fault_rca::ui::tray::{derive_tray_state, SingleInstanceGuard, TrayState};
use interface_fault_rca::writeback::report::export_handover;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = dispatch(&args);
    if code != 0 { std::process::exit(code); }
}

fn dispatch(args: &[String]) -> i32 {
    match args.first().map(|s| s.as_str()) {
        Some("diagnose") => cmd_diagnose(args),
        Some("report") => cmd_report(args),
        Some("clean") => cmd_clean(args),
        Some("version") => { println!("interface-fault-rca 0.1.0"); 0 }
        None | Some("daemon") => cmd_daemon(),
        Some(other) => { eprintln!("未知子命令: {other}（可用: diagnose / report / clean / version / daemon）"); 2 }
    }
}

fn parse_window(args: &[String]) -> Option<(i64, i64)> {
    let i = args.iter().position(|a| a == "--window")?;
    let spec = args.get(i + 1)?;
    let mut parts = spec.split('/');
    let a = parts.next()?;
    let b = parts.next()?;
    let to_ms = |s: &str| -> Option<i64> {
        chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp_millis())
            .or_else(|| s.parse::<i64>().ok())
    };
    Some((to_ms(a)?, to_ms(b)?))
}

fn flag(args: &[String], name: &str, default: &str) -> String {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned()).unwrap_or_else(|| default.to_string())
}

/// 载入接口实体绑定字典：优先配置文件，缺失时退回内置最小字典并提示。
fn load_dict(path: &str) -> (Dictionary, Option<String>) {
    let p = Path::new(path);
    if p.exists() {
        return match Dictionary::from_file(p) {
            Ok(d) => (d, None),
            Err(e) => (minimal_dict(), Some(format!("字典 {} 读取失败（{}），退回内置最小字典", path, e.detail))),
        };
    }
    (minimal_dict(), Some(format!("字典 {} 不存在，退回内置最小字典（仅含 2 条接口条目）", path)))
}

fn cmd_diagnose(args: &[String]) -> i32 {
    let (ws, we) = match parse_window(args) {
        Some(w) => w, None => { eprintln!("缺少 --window <起>/<止>（ISO8601 或毫秒时间戳）"); return 2; }
    };
    if we <= ws {
        eprintln!("窗口结束时间必须晚于开始时间");
        return 2;
    }
    let input = flag(args, "--input", "fixtures");
    let dict_path = flag(args, "--dict", "config/interface-dict.toml");
    let out_path = flag(args, "--out", "");
    let top_n = flag(args, "--top-n", "20").parse::<usize>().unwrap_or(20);

    let records: Vec<RawRecord> = match jsonl::load_dir(Path::new(&input)) {
        Ok(r) if !r.is_empty() => r,
        Ok(_) => { eprintln!("输入目录无数据: {input}"); return 3; }
        Err(e) => { eprintln!("读输入失败（{}）: {}", e.code.as_str(), e.detail); return 3; }
    };
    let (dict, dict_warn) = load_dict(&dict_path);
    if let Some(w) = dict_warn { eprintln!("提示: {w}"); }

    let cfg = Config::from_env();
    let opts = DiagnoseOptions {
        window_start_ms: ws,
        window_end_ms: we,
        baseline_minutes: cfg.baseline.window_minutes,
        baseline: cfg.baseline.clone(),
        dict: dict.clone(),
        use_model: !cfg.model.base_url.is_empty(),
        request_id: format!("diagnose-{}", ws),
    };

    // 模型端点配了才走模型路径；没配就走确定性基线路径（报告里显式标注降级）
    let client = ModelGatewayClient::new(cfg.model.clone());
    let gateway: Option<&dyn ModelGateway> = if opts.use_model { Some(&client) } else { None };

    let outcome = match diagnose(&opts, &records, gateway) {
        Ok(o) => o,
        Err(e) => { eprintln!("判定失败（{}）: {}", e.code.as_str(), e.detail); return 4; }
    };
    let report = &outcome.report;

    // 结论先落本地台账（candidate_ranked 为止，不发任何出站请求）
    let db_path = cfg.data_dir.join("ledger.sqlite");
    let mut ledger_written = 0usize;
    match IndexDb::open(&db_path) {
        Ok(db) => {
            let ids = record_candidates(&db, &outcome.attribution, ws, we, &dict.version, &chrono::Utc::now().to_rfc3339());
            ledger_written = ids.len();
        }
        Err(e) => eprintln!("提示: 台账写入失败（{}）: {}", e.code.as_str(), e.detail),
    }

    println!("=== 接口故障根因定位 · diagnose ===");
    println!("窗口: {} ~ {}", format_ms(ws), format_ms(we));
    println!("基线窗口: 前 {} 分钟（{} ~ {}）", report.baseline_minutes, format_ms(report.baseline_start_ms), format_ms(ws));
    println!("字典版本: {}｜输入: {}｜原始行 {} → 事件表 {} 行（窗口 {} / 基线 {}）",
        report.dict_version, input, report.total_rows, report.total_rows, report.current_rows, report.baseline_rows);
    for st in &report.sources {
        println!("  来源 {}（{}）：尝试 {} / 入库 {} / 拒绝 {} / 脏行 {}",
            st.source, st.event_kind, st.attempted, st.ingested, st.rejected, st.dirty_lines);
    }
    println!("候选 Top-{}（按综合得分排序：延迟绝对值 {:.2} + 劣化倍数 {:.2} + 错误率 {:.2}）：",
        report.candidates.len().min(top_n), report.candidates.first().map(|c| c.score_breakdown.latency_component).unwrap_or(0.0),
        report.candidates.first().map(|c| c.score_breakdown.ratio_component).unwrap_or(0.0),
        report.candidates.first().map(|c| c.score_breakdown.error_component).unwrap_or(0.0));
    for (i, c) in report.candidates.iter().take(top_n).enumerate() {
        println!("  {}. {} | {} | 劣化 {:.2} 倍 | P95 {:.0}ms（基线 {:.0}ms） | 得分 {:.3} | 形态 {} | 状态 candidate_ranked",
            i + 1, c.interface_code, c.verdict, c.degrade_ratio, c.p95_ms, c.baseline_p95_ms, c.score, c.change_shape);
        println!("     依据: {}", c.reasoning_summary);
        println!("     证据行: {}", c.evidence_refs.join(", "));
        if let Some(e) = c.evidence.first() {
            println!("     原文摘要: {}", e.summary);
        }
    }
    if !report.dropped.is_empty() {
        println!("未进候选 {} 条（附原因码，可回查）：", report.dropped.len());
        for d in report.dropped.iter().take(20) {
            println!("  - {} | {} | {}", d.interface_code, d.verdict, d.reason_code);
        }
    }
    if !report.log_templates.is_empty() {
        println!("错误日志模板 Top-{}：", report.log_templates.len().min(10));
        for g in report.log_templates.iter().take(10) {
            println!("  - {} 次 | 首现 {} | {}", g.count, format_ms(g.first_ts_ms), g.template);
        }
    }
    if report.degraded {
        println!("【{}】，以下为规则基线结论", report.degrade_reason);
    }
    println!("台账：候选 {} 条，本次新增 {} 条（status=candidate_ranked；未建单、无出站请求）",
        report.candidates.len(), ledger_written);

    if !out_path.is_empty() {
        match write_report(&out_path, report) {
            Ok(p) => println!("报告已落盘: {}", p),
            Err(e) => { eprintln!("报告落盘失败（{}）: {}", e.code.as_str(), e.detail); return 5; }
        }
    }
    0
}

/// 落 JSON 产物（可打开看、可被下游消费）
fn write_report(path: &str, report: &interface_fault_rca::diagnose::DiagnoseReport) -> Result<String> {
    let p = Path::new(path);
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| {
                AppError::new(ErrorCode::SourceUnavailable, format!("建立产物目录失败: {e}"))
            })?;
        }
    }
    let json = serde_json::to_string_pretty(report)
        .map_err(|e| AppError::new(ErrorCode::SchemaMismatch, format!("报告序列化失败: {e}")))?;
    std::fs::write(p, json + "\n").map_err(|e| {
        AppError::new(ErrorCode::SourceUnavailable, format!("写 {} 失败: {e}", p.display()))
    })?;
    Ok(p.display().to_string())
}

/// 时间显示用院内墙钟（UTC+8），与值班人看表一致
fn format_ms(ms: i64) -> String {
    let off = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.with_timezone(&off).format("%Y-%m-%d %H:%M:%S%z").to_string())
        .unwrap_or_else(|| ms.to_string())
}

fn cmd_report(args: &[String]) -> i32 {
    let (ws, we) = match parse_window(args) { Some(w) => w, None => { eprintln!("缺少 --window"); return 2; } };
    let cfg = Config::from_env();
    let db = match IndexDb::open(&cfg.data_dir.join("ledger.sqlite")) {
        Ok(d) => d, Err(e) => { eprintln!("台账打开失败: {e}"); return 4; }
    };
    let entries = db.ledger_all();
    let degraded = entries.iter().any(|e| e.degraded);
    let (txt, md) = export_handover(Path::new("docs"), ws, we, &entries, degraded);
    println!("{}", std::fs::read_to_string(&txt).unwrap_or_default());
    println!("已导出交班摘要: {txt} / {md}");
    0
}

fn cmd_clean(args: &[String]) -> i32 {
    let dry = args.iter().any(|a| a == "--dry-run");
    let cfg = Config::from_env();
    let root = &cfg.data_dir;
    println!("=== 保留期清理 {} ===", if dry { "预览（dry-run）" } else { "执行" });
    println!("保留期: {} 天；分区总量上限: {} 字节", cfg.retention.days, cfg.retention.max_partition_bytes);
    let mut found = 0;
    if let Ok(rd) = std::fs::read_dir(root) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                let name = p.file_name().and_then(|x| x.to_str()).unwrap_or("");
                if name.chars().nth(4) == Some('-') && name.len() == 10 {
                    println!("  将清理分区: {name}");
                    found += 1;
                    if !dry { let _ = std::fs::remove_dir_all(&p); }
                }
            }
        }
    }
    if found == 0 { println!("当前无过期分区（按 source × 日期分区，保留期 {} 天）", cfg.retention.days); }
    0
}

fn cmd_daemon() -> i32 {
    let guard = SingleInstanceGuard::new();
    if !guard.try_acquire() {
        eprintln!("已有实例在运行（RepeatedStart），拒绝启动第二份采集");
        return 6;
    }
    let cfg = Config::from_env();
    match cfg.validate(Strictness::Strict) {
        Ok(_) => {}
        Err(e) => { eprintln!("启动失败: {e}"); guard.release(); return 7; }
    }
    let tray = derive_tray_state(false, 0);
    println!("常驻启动：托盘状态 {}", match tray { TrayState::Gray => "灰(未采集)", TrayState::Green => "绿(正常)", TrayState::Amber => "琥珀(待确认)" });
    println!("配置摘要（脱敏）: {}", cfg.redacted_summary());
    guard.release();
    0
}
