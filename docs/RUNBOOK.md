# 接口故障根因定位 · 运行手册（唯一入口说明）

> 本文件是**唯一入口说明**：装完、跑起来、核对结论，只看这一份。
> 值班场景：夜班接口告警后，在值班机上跑一条命令，得到「昨夜哪条接口先坏的」，
> 每条结论都带**可回查的证据行 ID**；确认后由人工走运维工单系统建单。

---

## 1. 一句话说明

```
读三路来源（ESB 流水 / 错误日志 / 指标）→ 统一成一张运行事件表 → 切证据窗口与基线窗口
→ 按接口算劣化倍数并排序 → 证据守门逐条复核 → 出带证据行 ID 的判定报告（JSON 产物）
```

判定结果**只到 `candidate_ranked`**：程序不自动建单、不改配置、不重启服务。

---

## 2. 环境准备

### 2.1 依赖

| 项 | 要求 | 校验命令 |
|---|---|---|
| Rust 工具链 | 1.75 及以上（含 `cargo`） | `cargo --version` |
| 磁盘 | 约 300 MB（构建产物在 `target/`） | `df -h .` |
| 网络 | 仅构建期需要（拉 crates）；运行期不连公网 | — |

本产品是**单 exe + config 目录 + 环境变量**的形态，不装中心服务、不装数据库、不开 HTTP 端口。

### 2.2 构建

```bash
cd interface-fault-rca
cargo build --release          # 产物：target/release/interface-fault-rca
```

### 2.3 配置（凭据一律走环境变量）

只有接真实院内系统时才需要；跑内置样例（`fixtures/night/`）**不需要任何凭据**。

| 环境变量 | 用途 | 缺失时的行为 |
|---|---|---|
| `ESB_AUDIT_DB_HOST` / `_PORT` / `_USER` / `_PASSWORD` / `_NAME` | 院内 ESB 流水库（MySQL wire，只读账号） | `diagnose` 走文件投递形态，不报错 |
| `PROM_BASE_URL` / `PROM_BEARER_TOKEN` | 主机与中间件指标端点 | 同上 |
| `ESB_LOG_DIR` / `APP_LOG_DIR` | 应用与网关错误日志目录（只读） | 同上 |
| `OPS_TICKET_BASE_URL` / `OPS_TICKET_TOKEN` | 运维工单系统（唯一的出站写） | 未确认前程序**不会**发出站请求 |
| `MODEL_BASE_URL` / `MODEL_API_KEY` / `MODEL_NAME` | 归因模型端点（OpenAI-compatible） | 未配置 → 走确定性基线，报告里显式标 `degraded=true` |
| `DATA_DIR` | 本地台账与数据目录（默认 `data`） | — |
| `BASELINE_WINDOW_MINUTES` | 基线窗口长度，默认 `30` | — |
| `BASELINE_DEGRADE_RATIO` | 劣化倍数阈值，默认 `1.5` | — |
| `MIN_EVIDENCE_REFS` | 单条候选最少证据引用数，默认 `1` | — |

完整清单见 `.env.example`；程序启动即检查缺项，缺哪个会指名报哪个环境变量，**不会**静默退回示例数据。

---

## 3. 唯一入口命令

```bash
./target/release/interface-fault-rca diagnose \
  --window <起>/<止> \
  --input <投递目录或文件> \
  --dict  config/interface-dict.toml \
  --out   out/diagnose-<日期>-<场景>.json
```

### 3.1 子命令

| 子命令 | 作用 | 是否需要凭据 |
|---|---|---|
| `diagnose` | 跑一次判定，终端回显 + 可选落 JSON 产物 | 否（文件投递形态即可） |
| `report` | 从本地台账导出交班摘要（`docs/handover.txt` / `.md`） | 否 |
| `clean` | 按保留期清理过期分区（`--dry-run` 可预览） | 否 |
| `daemon` | 常驻采集模式（需真实凭据；缺项直接报错退出） | 是 |
| `version` | 打印版本号 | 否 |

### 3.2 参数与默认值

| 参数 | 必填 | 默认值 | 说明 |
|---|---|---|---|
| `--window <起>/<止>` | **是** | — | 证据窗口。格式 `RFC3339/RFC3339` 或 `毫秒时间戳/毫秒时间戳`，两者用 `/` 分隔；止必须晚于起 |
| `--input <路径>` | 否 | `fixtures` | 投递目录（读其下全部 `*.jsonl`）或单个 `.jsonl` 文件 |
| `--dict <路径>` | 否 | `config/interface-dict.toml` | 接口实体绑定字典（ESB 接口登记表口径）；找不到时退回内置最小字典并提示 |
| `--out <路径>` | 否 | 不落盘 | 判定报告 JSON 的落盘路径；父目录自动创建 |

### 3.3 输入样例

`--input` 指向的目录里，每行一个 JSON 对象。三路来源靠文件名或行内 `source` 区分：

| 文件名 | 来源 | 关键字段 |
|---|---|---|
| `esb_audit-flow.jsonl` | ESB 接口调用流水（MySQL 流水表导出形态） | `interface_code` `service_name` `vendor_name` `host` `request_time` `response_code` `duration_ms` `caller_system` |
| `app_log-error.jsonl` | 应用 / 网关错误日志 | `timestamp` `level` `message` `stack_head` `interface_code` |
| `prom-metric.jsonl` | Prometheus 指标序列 | `__ts` `__name` `__value` `labels` `interface_code` |

一行流水的真实形态（取自 `fixtures/night/esb_audit-flow.jsonl`）：

```json
{"source": "esb_audit", "interface_code": "ESB-ORDER-CREATE", "service_name": "住院医嘱开立服务", "vendor_name": "院内自研", "host": "esb-gw-01", "request_time": "2026-11-09T02:03:00.000+08:00", "response_code": 500, "duration_ms": 344, "caller_system": "HIS"}
```

时间戳统一归一到毫秒：unix 秒、unix 毫秒、RFC3339（带偏移）、MySQL `DATETIME(3)` 四种写法都收；
**无时区墙钟按 UTC 解释**（与 `esb_mysql` 适配器同口径）；缺时间戳或坏行直接报 `SchemaMismatch`，不塞 0 伪装成正常行。

---

## 4. 产物路径

| 产物 | 路径 | 说明 |
|---|---|---|
| 判定报告（JSON） | `--out` 指定，默认示例 `out/diagnose-2026-11-09-night.json` | 可打开看、可被下游消费；字段见 §5 |
| 本地结论台账 | `${DATA_DIR}/ledger.sqlite`（默认 `data/ledger.sqlite`） | 结论 + 证据引用 + 状态机 + 审计链，只增不改 |
| 交班摘要 | `docs/handover.txt` / `docs/handover.md` | `report` 子命令导出 |
| 金标准夹具 | `fixtures/gold/cases.jsonl` + `fixtures/gold/SOURCES.md` | 52 例判定基准（含来源文件与文号） |

---

## 5. 如何核对字段（报告 JSON）

| 字段 | 含义 | 怎么核对 |
|---|---|---|
| `window_start_ms` / `window_end_ms` | 证据窗口（毫秒） | 与 `--window` 传入的时刻一致（注意单位是毫秒） |
| `baseline_start_ms` / `baseline_minutes` | 基线窗口 = 窗口前 N 分钟 | `baseline_start_ms + baseline_minutes×60000 == window_start_ms` |
| `sources[]` | 每路来源的采集统计 | `attempted = ingested + rejected + dirty_lines`；`rejected_examples` 给出被拒原因（如 `AmbiguousInterface`） |
| `candidates[].verdict` | 判定结论 | 五取一：`接口自身劣化` / `下游依赖饱和` / `厂商返回异常` / `调用量突增` / `证据不足` |
| `candidates[].degrade_ratio` | 劣化倍数 | 当前窗口 P95 ÷ 基线 P95；应 > `BASELINE_DEGRADE_RATIO`（默认 1.5） |
| `candidates[].p95_ms` / `baseline_p95_ms` | 判定依据的两个数 | `p95_ms ÷ baseline_p95_ms == degrade_ratio`（允许末位误差） |
| `candidates[].score` / `score_breakdown` | 排序综合得分与分项 | `score == latency_component + ratio_component + error_component + host_component` |
| `candidates[].change_shape` | 劣化形态 | `阶梯劣化` / `尖刺` / `缓慢爬升` / `基线抖动` / `稳定` |
| `candidates[].evidence_refs[]` | 证据行 ID（`flow#a0041` 形态） | 每条都应能在 `candidates[].evidence[].row_id` 里找到同名字段 |
| `candidates[].evidence[]` | 证据原文锚点 | `row_id` / `ts_ms` / `source` / `event_kind` / `summary`；`summary` 是脱敏摘要，不是完整日志正文 |
| `dropped[].reason_code` | 没进候选的原因码 | `VERDICT_INSUFFICIENT`（判定为证据不足）/ `EVIDENCE_MISSING`（判定不带证据引用）/ `OUTSIDE_WINDOW` / `NONEXISTENT_REF` / `UNBOUND_REF` |
| `degraded` / `degrade_reason` | 是否降级、为什么 | `true` 表示模型端点不可达，本次结论由**确定性基线**给出 |
| `log_templates[]` | 错误日志模板归并 | 同一类错误出现次数与首次时间；模板里的数字已替换为 `<N>` |

**三条最常见的人工核对动作**：

1. 结论对不对 → 看 `candidates[].p95_ms` 与 `baseline_p95_ms`，自己算一遍倍数；
2. 证据真不真 → 拿 `evidence_refs[0]` 去原始来源按 `ts_ms` 核那一行；
3. 为什么没给我 → 看 `dropped[]` 的 `reason_code` 与 `reason`。

---

## 6. 一次真实运行（完整回显）

命令（在项目目录下执行）：

```bash
cargo run --release -- diagnose \
  --window 2026-11-09T02:00:00+08:00/2026-11-09T02:30:00+08:00 \
  --input fixtures/night \
  --dict config/interface-dict.toml \
  --out out/diagnose-2026-11-09-night.json
```

真实回显（逐字，未改写）：

```text
=== 接口故障根因定位 · diagnose ===
窗口: 2026-11-09 02:00:00+0800 ~ 2026-11-09 02:30:00+0800
基线窗口: 前 30 分钟（2026-11-09 01:30:00+0800 ~ 2026-11-09 02:00:00+0800）
字典版本: v1｜输入: fixtures/night｜原始行 118 → 事件表 118 行（窗口 68 / 基线 50）
  来源 app_log（log）：尝试 8 / 入库 8 / 拒绝 0 / 脏行 0
  来源 esb_audit（flow）：尝试 80 / 入库 80 / 拒绝 0 / 脏行 0
  来源 prom（metric）：尝试 30 / 入库 30 / 拒绝 0 / 脏行 0
候选 Top-2（按综合得分排序：延迟绝对值 0.20 + 劣化倍数 0.11 + 错误率 0.06）：
  1. EXT-HIS-PATIENT-BASE | 接口自身劣化 | 劣化 3.05 倍 | P95 672ms（基线 220ms） | 得分 0.369 | 形态 阶梯劣化 | 状态 candidate_ranked
     依据: P95 672ms / 基线 220ms = 3.05倍
     证据行: flow#a0043, flow#a0047, flow#a0051, flow#a0055, flow#a0059, flow#a0063, flow#a0067, flow#a0071, flow#a0075, flow#a0079
     原文摘要: flow#a0043 @1794160800000 EXT-HIS-PATIENT-BASE [rc=200 dur=610ms]
  2. ESB-ORDER-CREATE | 接口自身劣化 | 劣化 3.50 倍 | P95 368ms（基线 105ms） | 得分 0.273 | 形态 阶梯劣化 | 状态 candidate_ranked
     依据: P95 368ms / 基线 105ms = 3.50倍
     证据行: flow#a0041, flow#a0045, flow#a0049, flow#a0053, flow#a0057, flow#a0061, flow#a0065, flow#a0069, flow#a0073, flow#a0077
     原文摘要: flow#a0041 @1794160800000 ESB-ORDER-CREATE [rc=200 dur=340ms]
未进候选 2 条（附原因码，可回查）：
  - ESB-SETTLE-UPLOAD | 证据不足 | VERDICT_INSUFFICIENT
  - ESB-REPORT-QUERY | 证据不足 | VERDICT_INSUFFICIENT
错误日志模板 Top-5：
  - 3 次 | 首现 2026-11-09 02:01:00+0800 | upstream order-service call timed out after <N>ms
  - 2 次 | 首现 2026-11-09 02:03:00+0800 | vendor returned <N> from patient index endpoint
  - 1 次 | 首现 2026-11-09 02:05:00+0800 | connection pool exhausted for order-service
  - 1 次 | 首现 2026-11-09 02:02:00+0800 | 结算清单上传被拒：质控校验不通过（主诊断唯一性）
  - 1 次 | 首现 2026-11-09 02:05:00+0800 | 结算清单上传被拒：质控校验不通过（离院方式联动）
【模型未配置或不可用】，以下为规则基线结论
台账：候选 2 条，本次新增 2 条（status=candidate_ranked；未建单、无出站请求）
报告已落盘: out/diagnose-2026-11-09-night.json
```

**这段回显怎么读**：

- `ESB-ORDER-CREATE`（住院医嘱开立）P95 从 105ms 抬到 368ms（3.50 倍，阶梯劣化），对应错误日志模板
  `upstream order-service call timed out after <N>ms` 出现 3 次；
- `EXT-HIS-PATIENT-BASE`（院外 HIS 患者主索引）P95 从 220ms 抬到 672ms（3.05 倍），日志模板
  `vendor returned <N> from patient index endpoint` 出现 2 次 → 厂商侧返回异常；
- `ESB-SETTLE-UPLOAD` 出现 2 次「结算清单上传被拒：质控校验不通过」——这是**业务质控拒绝**，
  接口本身没有劣化（劣化倍数没过阈值），所以**不进候选**（`VERDICT_INSUFFICIENT`），
  这正是「不要把正常业务量当故障」的分界；
- `ESB-REPORT-QUERY` 全程正常，同样不进候选。

---

## 7. 判定质量自检

```bash
cargo test --test gold_regression      # 金标准逐例断言（52 例）
cargo test                               # 全量回归
```

金标准夹具的来源与文号见 `fixtures/gold/SOURCES.md`；语料正文逐字取自 `domain/cases/*.json`
（处方审核规范 / 结算清单填写规范 / 基金使用监督管理条例 / CHS-DRG 分组方案）。

---

## 8. 常见问题

| 现象 | 原因 | 处理 |
|---|---|---|
| `缺少 --window <起>/<止>` | 没给证据窗口 | 按 §3.1 补 `--window` |
| `输入目录无数据` | `--input` 指向的目录没有 `*.jsonl` | 确认投递路径；目录必须直接放 `.jsonl`（不再往下递归） |
| `读输入失败（SchemaMismatch）` | 来源行坏 JSON 或缺时间戳 | 按回显里的「第 N 行」定位；程序不静默丢弃脏行 |
| `字典 ... 不存在，退回内置最小字典` | `--dict` 路径不对 | 用 `config/interface-dict.toml`；内置字典只有 2 条接口条目，命中率会低 |
| 报告里 `degraded=true` | 模型端点不可达 | 结论仍可用（确定性基线路径）；要模型路径就配 `MODEL_BASE_URL` / `MODEL_API_KEY` |
| 台账「本次新增 0 条」 | 同一窗口重复跑，去重键命中 | 幂等设计，不是故障；用 `report` 子命令读已有台账 |
