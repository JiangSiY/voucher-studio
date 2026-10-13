#!/usr/bin/env bash
# 发票场景探测器（**只读**）——挂账 #30 的触发条件自查。
#
# 背景：2026-09 基准 102 张里增值税发票占比 0%，④发票版式模板当时判断"没有对象、不做"，
# 登记为触发式挂账：**真实数据出现发票场景再评估**。本脚本就是那个"触发传感器"——
# 想起来了、或感觉最近开始收发票了，跑一次看看。
#
# 用法：bash scripts/probe-invoice-signal.sh
# 行为：只读打开 voucher.db（?mode=ro），按月份统计带发票信号的凭证数，不改任何数据。
set -euo pipefail

DB="$HOME/Library/Application Support/VoucherStudio/voucher.db"
[ -f "$DB" ] || { echo "找不到库文件：$DB"; exit 1; }

echo "=== 发票信号按月分布（近 12 个月；信号=OCR 文本含 发票/价税合计/发票代码）==="
sqlite3 "file:${DB}?mode=ro" -readonly <<'SQL'
.mode column
.headers on
SELECT
  substr(v.voucher_date, 1, 7)                                        AS 月份,
  count(*)                                                            AS 凭证数,
  sum(CASE WHEN a.ocr_text LIKE '%发票%'
            OR a.ocr_text LIKE '%价税合计%'
            OR a.ocr_text LIKE '%发票代码%' THEN 1 ELSE 0 END)         AS 发票信号数
FROM vouchers v JOIN attachments a ON a.voucher_id = v.id
WHERE v.voucher_date >= date('now', '-12 months')
GROUP BY 1 ORDER BY 1 DESC;
SQL

TOTAL=$(sqlite3 "file:${DB}?mode=ro" -readonly \
  "SELECT count(*) FROM vouchers v JOIN attachments a ON a.voucher_id = v.id
   WHERE v.voucher_date >= date('now','-12 months')
     AND (a.ocr_text LIKE '%发票%' OR a.ocr_text LIKE '%价税合计%' OR a.ocr_text LIKE '%发票代码%');")

echo ""
echo "=== 判定 ==="
if [ "${TOTAL}" -ge 10 ]; then
  echo "近 12 个月发票信号 ${TOTAL} 张 —— 已达触发条件。带上这条命令的输出找 AI，评估④发票版式模板。"
elif [ "${TOTAL}" -ge 1 ]; then
  echo "近 12 个月发票信号 ${TOTAL} 张 —— 已出现苗头。继续攒，到 10 张以上再评估④发票版式模板。"
else
  echo "近 12 个月无发票信号 —— 未触发（④保持不做）。"
fi
