# 金标准夹具来源表（每例的来源文件与文号）

> 语料正文逐字拷贝自 `fixtures/gold/corpus/*.json`（与仓库 `domain/cases/*.json` 同源同内容）；
> 期望结论由人工依据语料规则与 `spec.md` 判定纪律写出，**未**运行被测代码反推。
> 金标准回归测试 `tests/gold_regression.rs` 会反向校验 payload 与语料逐字一致。

## 判定口径（为什么有的案例是故障、有的不是）

本产品监控的是院内 ESB 接口接缝，其中一部分接口承载**规范强制的业务判定**：

- 《医疗机构处方审核规范》：所有处方经审核通过后方可进入划价收费和调配环节，未经审核通过不得收费调配；
- 《医疗保障基金结算清单填写规范》：清单是医保结算唯一依据，DRG/DIP 分组全部基于清单而非首页；
- 《医疗保障基金使用监督管理条例》第38条：违规费用须逐条判定并可追溯；
- 《CHS-DRG分组方案》（医保办发〔2019〕36号）：分组结果须回传医保办。

因此判定口径是：

| 语料案例的期望 | 该接口在观测段的表现 | 期望结论 |
|---|---|---|
| 规则要求必须给出否定结论（拦截=true / 质控不通过 / 违规≠null / 先期分组） | 承载该判定的接口劣化（超时/5xx） | **接口自身劣化**，进候选，带证据行 |
| 规则要求正常放行（拦截=false / 质控通过 / 违规=null / 常规分组） | 接口表现正常 | **不应判出**（业务正常量不是故障） |

边界条件类负样本是在同一语料场景上叠加一个边界条件（基线样本不足、基线抖动、倍数未过阈值、
证据引用越界/不存在、实体未映射、接口码歧义、来源行非法），其期望结论来自判定纪律（`spec.md` §4 证据守门 / §5 基线退化处置），语料部分用于标注这条边界条件对应的业务场景。

丢弃原因码的读法：`VERDICT_INSUFFICIENT` = 判定本身是「证据不足」（劣化倍数没超阈值等）；
`EVIDENCE_MISSING` = 该判定不携带证据引用（基线样本不足 / 基线抖动过大时判定不给引用），
守门按「无证据引用」整条丢弃；`OUTSIDE_WINDOW` / `NONEXISTENT_REF` / `UNBOUND_REF` = 证据引用越界、
引用不存在、实体未映射；歧义接口码与坏行不产生候选，分别以 `AmbiguousInterface` 拒收与 `SchemaMismatch` 报错体现。

## 逐例来源

| 用例 | 语料文件 | 域案例 | 文号 | 条款 | 类型 |
|---|---|---|---|---|---|
| gold-drg-001 | domain/cases/drg_mdc_cases.json | drg-001 | 《国家医疗保障DRG分组与付费技术规范》《CHS-DRG分组方案》（医保办发〔2019〕36号） | CHS-DRG 分组逻辑 §1 分组三步法 / §2 先期分组（Pre-MDC） | 应判不出 |
| gold-drg-002 | domain/cases/drg_mdc_cases.json | drg-002 | 《国家医疗保障DRG分组与付费技术规范》《CHS-DRG分组方案》（医保办发〔2019〕36号） | CHS-DRG 分组逻辑 §1 分组三步法 / §2 先期分组（Pre-MDC） | 应判不出 |
| gold-drg-003 | domain/cases/drg_mdc_cases.json | drg-003 | 《国家医疗保障DRG分组与付费技术规范》《CHS-DRG分组方案》（医保办发〔2019〕36号） | CHS-DRG 分组逻辑 §1 分组三步法 / §2 先期分组（Pre-MDC） | — |
| gold-drg-004 | domain/cases/drg_mdc_cases.json | drg-004 | 《国家医疗保障DRG分组与付费技术规范》《CHS-DRG分组方案》（医保办发〔2019〕36号） | CHS-DRG 分组逻辑 §1 分组三步法 / §2 先期分组（Pre-MDC） | — |
| gold-drg-005 | domain/cases/drg_mdc_cases.json | drg-005 | 《国家医疗保障DRG分组与付费技术规范》《CHS-DRG分组方案》（医保办发〔2019〕36号） | CHS-DRG 分组逻辑 §1 分组三步法 / §2 先期分组（Pre-MDC） | — |
| gold-drg-006 | domain/cases/drg_mdc_cases.json | drg-006 | 《国家医疗保障DRG分组与付费技术规范》《CHS-DRG分组方案》（医保办发〔2019〕36号） | CHS-DRG 分组逻辑 §1 分组三步法 / §2 先期分组（Pre-MDC） | — |
| gold-drg-007 | domain/cases/drg_mdc_cases.json | drg-007 | 《国家医疗保障DRG分组与付费技术规范》《CHS-DRG分组方案》（医保办发〔2019〕36号） | CHS-DRG 分组逻辑 §1 分组三步法 / §2 先期分组（Pre-MDC） | — |
| gold-drg-008 | domain/cases/drg_mdc_cases.json | drg-008 | 《国家医疗保障DRG分组与付费技术规范》《CHS-DRG分组方案》（医保办发〔2019〕36号） | CHS-DRG 分组逻辑 §1 分组三步法 / §2 先期分组（Pre-MDC） | 应判不出 |
| gold-drg-009 | domain/cases/drg_mdc_cases.json | drg-009 | 《国家医疗保障DRG分组与付费技术规范》《CHS-DRG分组方案》（医保办发〔2019〕36号） | CHS-DRG 分组逻辑 §1 分组三步法 / §2 先期分组（Pre-MDC） | 应判不出 |
| gold-drg-010 | domain/cases/drg_mdc_cases.json | drg-010 | 《国家医疗保障DRG分组与付费技术规范》《CHS-DRG分组方案》（医保办发〔2019〕36号） | CHS-DRG 分组逻辑 §1 分组三步法 / §2 先期分组（Pre-MDC） | 应判不出 |
| gold-fraud-001 | domain/cases/fraud_review_cases.json | fraud-001 | 《医疗保障基金使用监督管理条例》（国务院令第735号）第38条、第40条 | 医保违规负面清单 §3 违规行为判定细化 / §4 智能审核规则条目 | — |
| gold-fraud-002 | domain/cases/fraud_review_cases.json | fraud-002 | 《医疗保障基金使用监督管理条例》（国务院令第735号）第38条、第40条 | 医保违规负面清单 §3 违规行为判定细化 / §4 智能审核规则条目 | 应判不出 |
| gold-fraud-003 | domain/cases/fraud_review_cases.json | fraud-003 | 《医疗保障基金使用监督管理条例》（国务院令第735号）第38条、第40条 | 医保违规负面清单 §3 违规行为判定细化 / §4 智能审核规则条目 | — |
| gold-fraud-004 | domain/cases/fraud_review_cases.json | fraud-004 | 《医疗保障基金使用监督管理条例》（国务院令第735号）第38条、第40条 | 医保违规负面清单 §3 违规行为判定细化 / §4 智能审核规则条目 | — |
| gold-fraud-005 | domain/cases/fraud_review_cases.json | fraud-005 | 《医疗保障基金使用监督管理条例》（国务院令第735号）第38条、第40条 | 医保违规负面清单 §3 违规行为判定细化 / §4 智能审核规则条目 | — |
| gold-fraud-006 | domain/cases/fraud_review_cases.json | fraud-006 | 《医疗保障基金使用监督管理条例》（国务院令第735号）第38条、第40条 | 医保违规负面清单 §3 违规行为判定细化 / §4 智能审核规则条目 | — |
| gold-fraud-007 | domain/cases/fraud_review_cases.json | fraud-007 | 《医疗保障基金使用监督管理条例》（国务院令第735号）第38条、第40条 | 医保违规负面清单 §3 违规行为判定细化 / §4 智能审核规则条目 | — |
| gold-fraud-008 | domain/cases/fraud_review_cases.json | fraud-008 | 《医疗保障基金使用监督管理条例》（国务院令第735号）第38条、第40条 | 医保违规负面清单 §3 违规行为判定细化 / §4 智能审核规则条目 | — |
| gold-fraud-009 | domain/cases/fraud_review_cases.json | fraud-009 | 《医疗保障基金使用监督管理条例》（国务院令第735号）第38条、第40条 | 医保违规负面清单 §3 违规行为判定细化 / §4 智能审核规则条目 | — |
| gold-fraud-010 | domain/cases/fraud_review_cases.json | fraud-010 | 《医疗保障基金使用监督管理条例》（国务院令第735号）第38条、第40条 | 医保违规负面清单 §3 违规行为判定细化 / §4 智能审核规则条目 | — |
| gold-rx-001 | domain/cases/prescription_review_cases.json | rx-001 | 《医疗机构处方审核规范》（国卫办医发〔2018〕14号）配套口径 /《处方管理办法》（卫生部令第53号） | 处方审核规则体系 §3 特殊药品处方量限 / §4 审核流程与信息系统要求 | — |
| gold-rx-002 | domain/cases/prescription_review_cases.json | rx-002 | 《医疗机构处方审核规范》（国卫办医发〔2018〕14号）配套口径 /《处方管理办法》（卫生部令第53号） | 处方审核规则体系 §3 特殊药品处方量限 / §4 审核流程与信息系统要求 | 应判不出 |
| gold-rx-003 | domain/cases/prescription_review_cases.json | rx-003 | 《医疗机构处方审核规范》（国卫办医发〔2018〕14号）配套口径 /《处方管理办法》（卫生部令第53号） | 处方审核规则体系 §3 特殊药品处方量限 / §4 审核流程与信息系统要求 | — |
| gold-rx-004 | domain/cases/prescription_review_cases.json | rx-004 | 《医疗机构处方审核规范》（国卫办医发〔2018〕14号）配套口径 /《处方管理办法》（卫生部令第53号） | 处方审核规则体系 §3 特殊药品处方量限 / §4 审核流程与信息系统要求 | — |
| gold-rx-005 | domain/cases/prescription_review_cases.json | rx-005 | 《医疗机构处方审核规范》（国卫办医发〔2018〕14号）配套口径 /《处方管理办法》（卫生部令第53号） | 处方审核规则体系 §3 特殊药品处方量限 / §4 审核流程与信息系统要求 | — |
| gold-rx-006 | domain/cases/prescription_review_cases.json | rx-006 | 《医疗机构处方审核规范》（国卫办医发〔2018〕14号）配套口径 /《处方管理办法》（卫生部令第53号） | 处方审核规则体系 §3 特殊药品处方量限 / §4 审核流程与信息系统要求 | 应判不出 |
| gold-rx-007 | domain/cases/prescription_review_cases.json | rx-007 | 《医疗机构处方审核规范》（国卫办医发〔2018〕14号）配套口径 /《处方管理办法》（卫生部令第53号） | 处方审核规则体系 §3 特殊药品处方量限 / §4 审核流程与信息系统要求 | — |
| gold-rx-008 | domain/cases/prescription_review_cases.json | rx-008 | 《医疗机构处方审核规范》（国卫办医发〔2018〕14号）配套口径 /《处方管理办法》（卫生部令第53号） | 处方审核规则体系 §3 特殊药品处方量限 / §4 审核流程与信息系统要求 | — |
| gold-rx-009 | domain/cases/prescription_review_cases.json | rx-009 | 《医疗机构处方审核规范》（国卫办医发〔2018〕14号）配套口径 /《处方管理办法》（卫生部令第53号） | 处方审核规则体系 §3 特殊药品处方量限 / §4 审核流程与信息系统要求 | — |
| gold-rx-010 | domain/cases/prescription_review_cases.json | rx-010 | 《医疗机构处方审核规范》（国卫办医发〔2018〕14号）配套口径 /《处方管理办法》（卫生部令第53号） | 处方审核规则体系 §3 特殊药品处方量限 / §4 审核流程与信息系统要求 | 应判不出 |
| gold-qc-001 | domain/cases/settlement_qc_cases.json | qc-001 | 《医疗保障基金结算清单填写规范》（国家医保局） | 结算清单填报硬规则 §3 / 质控硬校验 §4 | — |
| gold-qc-002 | domain/cases/settlement_qc_cases.json | qc-002 | 《医疗保障基金结算清单填写规范》（国家医保局） | 结算清单填报硬规则 §3 / 质控硬校验 §4 | — |
| gold-qc-003 | domain/cases/settlement_qc_cases.json | qc-003 | 《医疗保障基金结算清单填写规范》（国家医保局） | 结算清单填报硬规则 §3 / 质控硬校验 §4 | — |
| gold-qc-004 | domain/cases/settlement_qc_cases.json | qc-004 | 《医疗保障基金结算清单填写规范》（国家医保局） | 结算清单填报硬规则 §3 / 质控硬校验 §4 | — |
| gold-qc-005 | domain/cases/settlement_qc_cases.json | qc-005 | 《医疗保障基金结算清单填写规范》（国家医保局） | 结算清单填报硬规则 §3 / 质控硬校验 §4 | 应判不出 |
| gold-qc-006 | domain/cases/settlement_qc_cases.json | qc-006 | 《医疗保障基金结算清单填写规范》（国家医保局） | 结算清单填报硬规则 §3 / 质控硬校验 §4 | — |
| gold-qc-007 | domain/cases/settlement_qc_cases.json | qc-007 | 《医疗保障基金结算清单填写规范》（国家医保局） | 结算清单填报硬规则 §3 / 质控硬校验 §4 | — |
| gold-qc-008 | domain/cases/settlement_qc_cases.json | qc-008 | 《医疗保障基金结算清单填写规范》（国家医保局） | 结算清单填报硬规则 §3 / 质控硬校验 §4 | 应判不出 |
| gold-bound-001 | domain/cases/prescription_review_cases.json | rx-002 | 《医疗机构处方审核规范》（国卫办医发〔2018〕14号）配套口径 /《处方管理办法》（卫生部令第53号） | 处方审核规则体系 §3 特殊药品处方量限 / §4 审核流程与信息系统要求 | 应判不出 |
| gold-bound-002 | domain/cases/settlement_qc_cases.json | qc-005 | 《医疗保障基金结算清单填写规范》（国家医保局） | 结算清单填报硬规则 §3 / 质控硬校验 §4 | 应判不出 |
| gold-bound-003 | domain/cases/fraud_review_cases.json | fraud-002 | 《医疗保障基金使用监督管理条例》（国务院令第735号）第38条、第40条 | 医保违规负面清单 §3 违规行为判定细化 / §4 智能审核规则条目 | 应判不出 |
| gold-bound-004 | domain/cases/drg_mdc_cases.json | drg-009 | 《国家医疗保障DRG分组与付费技术规范》《CHS-DRG分组方案》（医保办发〔2019〕36号） | CHS-DRG 分组逻辑 §1 分组三步法 / §2 先期分组（Pre-MDC） | 应判不出 |
| gold-bound-005 | domain/cases/prescription_review_cases.json | rx-006 | 《医疗机构处方审核规范》（国卫办医发〔2018〕14号）配套口径 /《处方管理办法》（卫生部令第53号） | 处方审核规则体系 §3 特殊药品处方量限 / §4 审核流程与信息系统要求 | 应判不出 |
| gold-bound-006 | domain/cases/settlement_qc_cases.json | qc-008 | 《医疗保障基金结算清单填写规范》（国家医保局） | 结算清单填报硬规则 §3 / 质控硬校验 §4 | 应判不出 |
| gold-bound-007 | domain/cases/prescription_review_cases.json | rx-010 | 《医疗机构处方审核规范》（国卫办医发〔2018〕14号）配套口径 /《处方管理办法》（卫生部令第53号） | 处方审核规则体系 §3 特殊药品处方量限 / §4 审核流程与信息系统要求 | 应判不出 |
| gold-bound-008 | domain/cases/drg_mdc_cases.json | drg-002 | 《国家医疗保障DRG分组与付费技术规范》《CHS-DRG分组方案》（医保办发〔2019〕36号） | CHS-DRG 分组逻辑 §1 分组三步法 / §2 先期分组（Pre-MDC） | 应判不出 |
| gold-bound-009 | domain/cases/fraud_review_cases.json | fraud-003 | 《医疗保障基金使用监督管理条例》（国务院令第735号）第38条、第40条 | 医保违规负面清单 §3 违规行为判定细化 / §4 智能审核规则条目 | 应拒绝 |
| gold-bound-010 | domain/cases/settlement_qc_cases.json | qc-004 | 《医疗保障基金结算清单填写规范》（国家医保局） | 结算清单填报硬规则 §3 / 质控硬校验 §4 | 应拒绝 |
| gold-bound-011 | domain/cases/fraud_review_cases.json | fraud-009 | 《医疗保障基金使用监督管理条例》（国务院令第735号）第38条、第40条 | 医保违规负面清单 §3 违规行为判定细化 / §4 智能审核规则条目 | 应拒绝 |
| gold-bound-012 | domain/cases/drg_mdc_cases.json | drg-005 | 《国家医疗保障DRG分组与付费技术规范》《CHS-DRG分组方案》（医保办发〔2019〕36号） | CHS-DRG 分组逻辑 §1 分组三步法 / §2 先期分组（Pre-MDC） | 应拒绝 |
| gold-bound-013 | domain/cases/settlement_qc_cases.json | qc-006 | 《医疗保障基金结算清单填写规范》（国家医保局） | 结算清单填报硬规则 §3 / 质控硬校验 §4 | 应报错 |
| gold-bound-014 | domain/cases/prescription_review_cases.json | rx-003 | 《医疗机构处方审核规范》（国卫办医发〔2018〕14号）配套口径 /《处方管理办法》（卫生部令第53号） | 处方审核规则体系 §3 特殊药品处方量限 / §4 审核流程与信息系统要求 | 应报错 |

## 口径统计

- 总例数：52
- 正样本（应判出）：27
- 负样本（应判不出 / 应拒绝 / 应报错）：25

锚点：`anchor_ms = %d`（2026-11-09 02:00:00 +08:00），行内 `t_min` 为相对锚点的分钟偏移；
基线段取 t_min ∈ [-10, -1]，观测段取 t_min ∈ [0, 3]，基线窗口长度 30 分钟。

