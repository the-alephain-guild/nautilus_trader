#!/bin/bash
# 第三版纸上运行的看护：进程异常退出即重启，时限到达即停。
#   ./atr_v3_run.sh maker all 172800
# journal 追加写入，重启不丢已有记录；心跳里的计数器每次重启归零，校验脚本按此处理。
# sandbox 模拟撮合，不下真实订单。
set -u
DIR="$(cd "$(dirname "$0")" && pwd)"
ARM="${1:?用法: atr_v3_run.sh maker|taker thick|all|lead_thin [秒]}"
RULES="${2:?用法: atr_v3_run.sh maker|taker thick|all|lead_thin [秒]}"
DURATION="${3:-172800}"
BIN="$DIR/target-v3/debug/examples/polymarket-atr-margin-sandbox"
LOGDIR="$DIR/v3_logs"; SUP="$LOGDIR/supervisor_${ARM}_${RULES}.log"
mkdir -p "$LOGDIR"; cd "$DIR" || exit 1
[ -x "$BIN" ] || { echo "$(date -u +%FT%TZ) 可执行文件不存在: $BIN" >> "$SUP"; exit 1; }
END=$(( $(date +%s) + DURATION ))
echo "=== $(date -u +%FT%TZ) 启动 arm=$ARM rules=$RULES 时长=${DURATION}s 至 $(date -u -r "$END" +%FT%TZ) 代理=${HTTPS_PROXY:-${https_proxy:-未设}}" >> "$SUP"
n=0
while :; do
  remain=$(( END - $(date +%s) )); [ "$remain" -le 30 ] && break
  n=$((n+1)); echo "$(date -u +%FT%TZ) 第 $n 次启动，剩余 ${remain}s" >> "$SUP"
  ATR_ARM="$ARM" ATR_RULES="$RULES" timeout "$remain" "$BIN" >> "$LOGDIR/${ARM}_${RULES}.log" 2>&1
  rc=$?; echo "$(date -u +%FT%TZ) 第 $n 次退出 rc=$rc（124=时限到达）" >> "$SUP"
  [ "$rc" -eq 124 ] && break
  sleep 10
done
echo "$(date -u +%FT%TZ) 看护结束，共 $n 次启动" >> "$SUP"
