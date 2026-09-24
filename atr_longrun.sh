#!/bin/bash
# 第一版纸上运行（只开 thick-lead 规则、推算口径、无真实成交）的看护脚本；
#   ./atr_longrun.sh            时长由 ATR_DURATION 控制（默认 1.6 天），二进制取 target/debug
# 已跑完 2026-09-23T01:57Z 至 09-24T11:57Z 一轮，留作 v2/v3 看护脚本的来源与对照。
#
# 为什么需要它：策略进程是长驻的，而 1.6 天里网络中断、场所限流或进程异常退出都
# 可能发生。决策日志是 append 打开的，重启不会截断已有记录，所以重启是安全的 ——
# 真正的风险是无人重启时静默停摆，而空缺在日志里看起来和「没有信号」一样。
#
# 运行模式：sandbox（模拟撮合）。订单不会送达场所，不涉及真实资金。
set -u

DIR="$(cd "$(dirname "$0")" && pwd)"
BIN="$DIR/target/debug/examples/polymarket-atr-margin-sandbox"
DURATION="${ATR_DURATION:-138240}"        # 1.6 天
LOGDIR="$DIR/longrun_logs"
SUP="$DIR/longrun_supervisor.log"

cd "$DIR" || exit 1
mkdir -p "$LOGDIR"
END=$(( $(date +%s) + DURATION ))

{
  echo "=== $(date -u +%FT%TZ) 启动看护 ==="
  echo "    可执行文件: $BIN"
  echo "    时长: ${DURATION}s（至 $(date -u -r "$END" +%FT%TZ)）"
  echo "    参数: 生产（未设 ATR_VERIFY → 60s ATR bar / 5 bars / ±20s 容差）"
  echo "    模式: sandbox 模拟撮合，不下真实订单"
  echo "    代理: ${HTTPS_PROXY:-${https_proxy:-未设}}"
} >> "$SUP"

attempt=0
while [ "$(date +%s)" -lt "$END" ]; do
  attempt=$((attempt + 1))
  remain=$(( END - $(date +%s) ))
  [ "$remain" -le 30 ] && break
  log="$LOGDIR/run_$(printf '%03d' "$attempt").log"
  echo "$(date -u +%FT%TZ) 第 $attempt 次启动，剩余 ${remain}s，日志 $log" >> "$SUP"
  # timeout 让程序在时限到达时收到终止信号；计数器已由心跳周期性落盘，
  # 故不依赖停止回调能否执行。
  timeout "$remain" "$BIN" >> "$log" 2>&1
  rc=$?
  echo "$(date -u +%FT%TZ) 第 $attempt 次退出，rc=${rc}（124=时限到达）" >> "$SUP"
  [ "$rc" -eq 124 ] && break
  sleep 10
done

echo "$(date -u +%FT%TZ) 看护结束，共 $attempt 次启动" >> "$SUP"
