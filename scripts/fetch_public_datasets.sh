#!/usr/bin/env bash
# 公开数据集脚本化下载（task 13）。断网降级：网络不可用时写 DEGRADED.md，
# 以可区分的退出码 10 结束（不报错崩溃），后续验收改用 gen_scale_data.py 合成数据。
# 重跑可断点续下：已存在且非空的文件跳过。
set -u

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/data/public"
mkdir -p "$OUT"

DEGRADED=0
MISSING=""
FETCHED=""

# 三个公开样例源（均 Apache-2.0）。URL 为示例占位，实际网络不可达时走降级分支。
download_one() {
  local name="$1" url="$2"
  local dest="$OUT/$name"
  if [ -s "$dest" ]; then
    FETCHED="$FETCHED\n- ${name}（已存在，跳过）"
    return 0
  fi
  # 短超时，失败即降级，不重试（对齐 Clash 兜底不重试纪律）
  if command -v curl >/dev/null 2>&1; then
    if curl -fsSL --connect-timeout 5 --max-time 20 -o "$dest" "$url" 2>/dev/null && [ -s "$dest" ]; then
      FETCHED="$FETCHED\n- ${name}（下载并校验可读）"
      return 0
    fi
  fi
  rm -f "$dest"
  MISSING="$MISSING\n- ${name}：$url"
  DEGRADED=1
  return 0
}

download_one "skywalking-apm-sample.json" "https://raw.githubusercontent.com/apache/skywalking/master/docs/en/samples/apm-sample.json"
download_one "prometheus-sample-series.txt" "https://raw.githubusercontent.com/prometheus/prometheus/main/documentation/examples/prometheus.yml"
download_one "public-log-sample.log" "https://raw.githubusercontent.com/elastic/examples/master/Common%20Data%20Formats/apache_logs/apache_logs"

if [ "$DEGRADED" -ne 0 ]; then
  {
    echo "# 公开数据集获取降级说明"
    echo
    echo "本环境网络不可达，以下公开数据集未能下载："
    printf "%b\n" "$MISSING"
    echo
    echo "## 后续验收替代方案"
    echo
    echo "案例集、golden 对拍与真实规模性能验收改用 \`scripts/gen_scale_data.py\` 生成的合成数据。"
    echo "合成数据带确定性种子与注入形态真值，字段严格对齐统一事件表。"
    echo
    echo "重跑本脚本可断点续下：已校验的文件不重复下载。"
  } > "$OUT/DEGRADED.md"
  printf "DEGRADED: 部分公开数据集缺失，已写 %s/DEGRADED.md\n" "$OUT"
  printf "%b\n" "$FETCHED"
  exit 10
fi

printf "OK: 三个公开数据集均已下载并校验可读\n"
printf "%b\n" "$FETCHED"
exit 0
