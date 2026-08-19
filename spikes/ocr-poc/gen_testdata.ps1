# Spike B 测试数据生成器：24 张合成凭证图 + ground truth
# 字典校验：从 rec 模型 ONNX 元数据中提取 character 字典，确保测试字符全在词表内
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

$projRoot = "F:\开发\voucher-studio"
$modelDir = "$projRoot\assets\ocr-models"
$recModel = "$modelDir\ch_PP-OCRv4_rec_infer.onnx"
$outDir = "$projRoot\spikes\ocr-poc\testdata"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
Get-ChildItem $outDir -File | Remove-Item -Force -ErrorAction SilentlyContinue

# ---- 1. 提取字典 ----
$bytes = [System.IO.File]::ReadAllBytes($recModel)
$needle = [System.Text.Encoding]::ASCII.GetBytes("character")
$found = -1
for ($i = $bytes.Length - 1; $i -ge 0; $i--) {
    if ($bytes[$i] -eq $needle[0]) {
        $ok = $true
        for ($j = 1; $j -lt $needle.Length; $j++) { if ($bytes[$i+$j] -ne $needle[$j]) { $ok = $false; break } }
        if ($ok) { $found = $i; break }
    }
}
if ($found -lt 0) { throw "rec 模型未找到 character 元数据" }
$p = $found + $needle.Length
$valueTag = $bytes[$p]; $p++     # protobuf value 字段 tag (0x12)
if ($valueTag -ne 0x12) { throw "字典元数据结构异常 (tag=$valueTag)" }
$len = 0; $shift = 0
while ($true) {
    $b = $bytes[$p]; $p++
    $len = $len -bor (($b -band 0x7F) -shl $shift)
    if (($b -band 0x80) -eq 0) { break }
    $shift += 7
}
$dictStr = [System.Text.Encoding]::UTF8.GetString($bytes, $p, $len)
$dictEntries = $dictStr -split "`n" | Where-Object { $_.Length -gt 0 }
$dictSet = [System.Collections.Generic.HashSet[string]]::new()
foreach ($c in $dictEntries) { [void]$dictSet.Add($c) }
Write-Output "字典条目数: $($dictEntries.Count)"

# ---- 2. 内容池（全部用极常用字） ----
$titles  = @("报销凭证", "费用报销单", "记账凭证", "原始凭证整理单", "支出凭证", "付款申请单")
$seqs    = @("001","002","003","004","005","006","007","008","009","010","011","012","013","014","015","016","017","018","019","020","021","022","023","024")
$amounts = @("金额：1234.56 元", "金额：98.00 元", "金额：4567.80 元", "金额：250.00 元", "金额：88.50 元", "金额：3010.40 元")
$descs   = @("摘要：办公用品采购", "摘要：差旅费报销", "摘要：员工餐费补贴", "摘要：打印耗材购买", "摘要：会议费用支出", "摘要：快递费结算")
$depts   = @("部门：财务部", "部门：行政部", "部门：市场部", "部门：人事部")
$persons = @("经办人：张三", "经办人：李四", "经办人：王五", "经办人：赵六")
$companys= @("单位：杭州某某科技有限公司", "单位：北京某某贸易有限公司", "单位：上海某某实业有限公司", "单位：广州某某网络公司")
$remarks = @("备注：已核对发票信息", "备注：附件共3张", "备注：本月结清", "备注：需领导审批")
$dates   = @("日期：2026-08-19", "日期：2026-08-18", "日期：2026-08-20", "日期：2026-07-31")
$modes   = @("方式：银行转账", "方式：现金支付", "方式：微信支付", "方式：支票")

# 覆盖校验（空格是 CTC 解码的特殊字符，追加在词表末尾，不在字典内——跳过）
$allStrings = $titles + $amounts + $descs + $depts + $persons + $companys + $remarks + $dates + $modes + $seqs
$missing = @()
foreach ($s in $allStrings) {
    foreach ($ch in $s.ToCharArray()) {
        if ([char]::IsWhiteSpace($ch)) { continue }
        if (-not $dictSet.Contains([string]$ch)) { $missing += $ch }
    }
}
$missing = $missing | Select-Object -Unique
if ($missing.Count -gt 0) { Write-Output ("词表缺失字符: " + ($missing -join " ")); throw "存在词表外字符，请调整内容池" }
Write-Output "内容池字符覆盖校验: 通过"

# ---- 3. 生成图片 ----
$fontFamilies = @("Microsoft YaHei", "SimHei", "SimSun")
$gt = @()
$rng = New-Object System.Random(20260819)

for ($i = 1; $i -le 24; $i++) {
    $idx = $i - 1
    $style = if ($i -le 12) { "form" } elseif ($i -le 16) { "twocol" } elseif ($i -le 20) { "small" } elseif ($i -le 22) { "rotated" } else { "large" }

    $W = if ($style -eq "large") { 900 } else { 800 }
    $H = if ($style -eq "large") { 640 } else { 560 }
    $bmp = New-Object System.Drawing.Bitmap($W, $H)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAlias
    $g.Clear([System.Drawing.Color]::White)

    $fam = $fontFamilies[$idx % 3]
    $bodySize = if ($style -eq "small") { 15 } else { 20 }
    $titleSize = if ($style -eq "small") { 24 } else { 30 }
    $fontBody = New-Object System.Drawing.Font($fam, $bodySize)
    $fontTitle = New-Object System.Drawing.Font($fam, $titleSize, [System.Drawing.FontStyle]::Bold)
    $black = [System.Drawing.Brushes]::Black
    $darkGray = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(60, 60, 60))

    if ($style -eq "rotated") {
        $rotAngle = 2
        if ($i -eq 22) { $rotAngle = -2 }
        $g.TranslateTransform(60, 60)
        $g.RotateTransform($rotAngle)
    }

    # 淡灰边框（部分图片）
    if ($idx % 2 -eq 0) {
        $pen = New-Object System.Drawing.Pen([System.Drawing.Color]::FromArgb(210, 210, 210), 1)
        $g.DrawRectangle($pen, 20, 20, $W - 44, $H - 44)
        $pen.Dispose()
    }

    $title = $titles[$idx % 6]
    $lines = @()
    $lines += $title
    $lines += "凭证编号：20260819-$($seqs[$idx])"
    $lines += $dates[$idx % 4]
    $lines += $amounts[$idx % 6]
    $lines += $descs[$idx % 6]
    $lines += $depts[$idx % 4]
    $lines += $persons[$idx % 4]
    $lines += $modes[$idx % 4]
    if ($style -eq "large" -or $style -eq "twocol") { $lines += $companys[$idx % 4] }
    if ($style -eq "large") { $lines += $remarks[$idx % 4] }

    $y = 40.0
    $lineNo = 0
    foreach ($ln in $lines) {
        if ($lineNo -eq 0) {
            # 标题居中
            $sz = $g.MeasureString($ln, $fontTitle)
            $g.DrawString($ln, $fontTitle, $black, [float](($W - $sz.Width) / 2), [float]$y)
            $y += $titleSize + 26
        } elseif ($style -eq "twocol" -and $ln -match "^(.*?：)(.*)$") {
            $label = $Matches[1]; $value = $Matches[2]
            $g.DrawString($label, $fontBody, $darkGray, 70, [float]$y)
            $g.DrawString($value, $fontBody, $black, 330, [float]$y)
            $y += $bodySize + 18
        } else {
            $g.DrawString($ln, $fontBody, $black, 56, [float]$y)
            $y += $bodySize + 18
        }
        $lineNo++
    }

    $g.ResetTransform()

    # 轻噪点：半透明灰点
    $noiseBrush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(30, 120, 120, 120))
    for ($n = 0; $n -lt 2200; $n++) {
        $nx = $rng.Next(0, $W); $ny = $rng.Next(0, $H)
        $g.FillRectangle($noiseBrush, $nx, $ny, 1, 1)
    }
    $noiseBrush.Dispose()

    $file = "v{0:d2}.png" -f $i
    $bmp.Save("$outDir\$file", [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    $fontBody.Dispose(); $fontTitle.Dispose()

    # GT：twocol 布局的「标签：值」会分成两个框、同行拼接，归一化（去空白）后仍一致
    $gt += [pscustomobject]@{ file = $file; lines = $lines }
    $idx++
}

$gtJson = @{ images = $gt } | ConvertTo-Json -Depth 4
[System.IO.File]::WriteAllText("$outDir\ground_truth.json", $gtJson, [System.Text.Encoding]::UTF8)
Write-Output "生成完成: $($gt.Count) 张图片 + ground_truth.json"
