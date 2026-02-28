#Requires -Version 5.1
# 性能对比: 微信小游戏 appbrand vs 当前SDK
# 用法:
#   .\perf_compare.ps1
#   .\perf_compare.ps1 -Loop 10 -Interval 3
#   .\perf_compare.ps1 -Loop 10 -Csv out.csv

param(
    [int]$Loop     = 1,
    [int]$Interval = 2,
    [string]$Csv   = ""
)

$MY_SDK = "com.minigame.androiddemo"
$script:activeWx = $null

# ── 工具函数 ─────────────────────────────────────────────

function adb_sh([string]$cmd) {
    (adb shell $cmd 2>$null) -replace "`r", ""
}

function ifval($a, $b) { if ($a) { $a } else { $b } }

function Get-ProcPid([string]$procName) {
    $r = adb_sh "pidof $procName"
    if (-not $r) { return "" }
    return ($r -split "\s+")[0].Trim()
}

# 返回所有 appbrand 进程列表（按CPU降序）
function Get-AppBrandProcs() {
    $lines = adb_sh "top -b -n 2 -d 1" | Where-Object { $_ -match "appbrand" }
    $seen = @{}
    foreach ($line in $lines) {
        $parts = $line.Trim() -split "\s+"
        if ($parts.Count -lt 12) { continue }
        $p = $parts[0]
        $seen[$p] = [PSCustomObject]@{
            procPid = $p
            name    = $parts[11]
            cpu     = [float]$parts[8]
            rss     = $parts[5]
            threads = "N/A"
        }
    }
    $result = @()
    foreach ($e in $seen.Values) {
        $thrLine = adb_sh "cat /proc/$($e.procPid)/status" |
                   Where-Object { $_ -match "^Threads:" } |
                   Select-Object -First 1
        if ($thrLine) { $e.threads = ($thrLine -split "\s+")[1].Trim() }
        $result += $e
    }
    return @($result | Sort-Object { -$_.cpu })
}

# 从 dumpsys meminfo 提取关键内存指标 (KB)
function Get-MemInfo([string]$target) {
    $raw = adb_sh "dumpsys meminfo $target"

    function PickVal([string]$pat, [int]$col) {
        $line = $raw | Where-Object { $_ -match $pat } | Select-Object -First 1
        if (-not $line) { return "N/A" }
        $parts = ($line.Trim() -split "\s+") | Where-Object { $_ -ne "" }
        if ($parts.Count -gt $col) { return $parts[$col] }
        return "N/A"
    }

    return [PSCustomObject]@{
        pss    = PickVal "TOTAL PSS:"   2
        native = PickVal "Native Heap:" 2
        java   = PickVal "Java Heap:"   2
        gfx    = PickVal "Graphics:"    1
        stack  = PickVal "^\s+Stack "   1
    }
}

function Get-Threads([string]$procPid) {
    $line = adb_sh "cat /proc/$procPid/status" |
            Where-Object { $_ -match "^Threads:" } | Select-Object -First 1
    if ($line) { return ($line -split "\s+")[1].Trim() }
    return "N/A"
}

# ── 打印工具 ─────────────────────────────────────────────

function Write-Header([string]$text) {
    $sep = "=" * 68
    Write-Host $sep -ForegroundColor Cyan
    Write-Host "  $text" -ForegroundColor Cyan
    Write-Host $sep -ForegroundColor Cyan
}

function Write-TR([string]$c1, [string]$c2, [string]$c3, [string]$Color = "White") {
    Write-Host ("|{0,-18}|{1,-22}|{2,-22}|" -f $c1, $c2, $c3) -ForegroundColor $Color
}

function Write-TSep {
    Write-Host ("|{0}|{1}|{2}|" -f ("-"*18), ("-"*22), ("-"*22))
}

# ── 单次采集 ─────────────────────────────────────────────

function Collect-Snapshot([int]$round) {
    $ts = Get-Date -Format "yyyy-MM-dd HH:mm:ss"
    Write-Header "第 $round 次采集  $ts"

    # ─ 微信 appbrand ─────────────────────────────────
    Write-Host ""
    Write-Host "[微信小游戏 - appbrand 沙箱进程]" -ForegroundColor Yellow
    $brands = Get-AppBrandProcs

    if ($brands.Count -eq 0) {
        Write-Host "  未检测到 appbrand 进程（小游戏未运行？）" -ForegroundColor DarkGray
        $script:activeWx = $null
    } else {
        $script:activeWx = $brands[0]   # CPU 最高 = 活跃游戏

        Write-Host ("|{0,-13}|{1,-7}|{2,-8}|{3,-10}|{4,-8}|{5,-7}|" -f `
            "进程", "CPU%", "RSS", "PSS(KB)", "Gfx(KB)", "线程") -ForegroundColor Yellow
        Write-Host ("|{0,-13}|{1,-7}|{2,-8}|{3,-10}|{4,-8}|{5,-7}|" -f `
            ("-"*13), ("-"*7), ("-"*8), ("-"*10), ("-"*8), ("-"*7))

        foreach ($b in $brands) {
            $mem   = Get-MemInfo $b.procPid
            $short = ($b.name -replace [regex]::Escape("com.tencent.mm:"), "")
            $tag   = if ($b.cpu -gt 5) { "*" } else { " " }
            $color = if ($b.cpu -gt 5) { "Green" } else { "DarkGray" }
            Write-Host ("|{0,-13}|{1,-7}|{2,-8}|{3,-10}|{4,-8}|{5,-7}|" -f `
                "$short$tag", "$($b.cpu)%", $b.rss, $mem.pss, $mem.gfx, $b.threads) `
                -ForegroundColor $color
        }
        Write-Host "  * = 活跃游戏（CPU最高）" -ForegroundColor DarkGray
    }

    # ─ 当前 SDK ──────────────────────────────────────
    Write-Host ""
    Write-Host "[当前SDK]  $MY_SDK" -ForegroundColor Yellow
    $myProcPid = Get-ProcPid $MY_SDK

    $myCpu = "N/A"; $myRss = "N/A"; $myThreads = "N/A"
    $myMem = [PSCustomObject]@{ pss="N/A"; native="N/A"; java="N/A"; gfx="N/A"; stack="N/A" }

    if (-not $myProcPid) {
        Write-Host "  进程未运行" -ForegroundColor DarkGray
    } else {
        $myTopLine = adb_sh "top -b -n 2 -d 1 -p $myProcPid" |
                     Where-Object { $_ -match $myProcPid } |
                     Select-Object -Last 1
        if ($myTopLine) {
            $p = $myTopLine.Trim() -split "\s+"
            $myCpu = if ($p.Count -ge 9) { "$($p[8])%" } else { "N/A" }
            $myRss = if ($p.Count -ge 6) { $p[5] }       else { "N/A" }
        }
        $myMem     = Get-MemInfo $myProcPid
        $myThreads = Get-Threads $myProcPid
        Write-Host ("  PID:{0}  CPU:{1}  RSS:{2}  PSS:{3}KB  线程:{4}" -f `
            $myProcPid, $myCpu, $myRss, $myMem.pss, $myThreads) -ForegroundColor Green
    }

    # ─ 对比表格 ──────────────────────────────────────
    Write-Host ""
    if ($script:activeWx -and $myProcPid) {
        $wx    = $script:activeWx
        $wxMem = Get-MemInfo $wx.procPid
        $wxCol = ($wx.name -replace [regex]::Escape("com.tencent.mm:"), "") + "(活跃)"

        Write-Host "[直接对比]" -ForegroundColor Cyan
        Write-TSep
        Write-TR "指标" $wxCol "当前SDK" -Color Yellow
        Write-TSep
        Write-TR "CPU%"            "$($wx.cpu)%"  $myCpu
        Write-TR "RSS (top)"       $wx.rss        $myRss
        Write-TR "PSS Total(KB)"   $wxMem.pss     $myMem.pss
        Write-TR "Native Heap(KB)" $wxMem.native  $myMem.native
        Write-TR "Java Heap(KB)"   $wxMem.java    $myMem.java
        Write-TR "Graphics(KB)"    $wxMem.gfx     $myMem.gfx
        Write-TR "Stack(KB)"       $wxMem.stack   $myMem.stack
        Write-TR "线程数"           $wx.threads    $myThreads
        Write-TSep

        if ($Csv) {
            "$ts,$($wx.cpu),$myCpu,$($wxMem.pss),$($myMem.pss),$($wxMem.native),$($myMem.native),$($wxMem.java),$($myMem.java),$($wxMem.gfx),$($myMem.gfx),$($wx.threads),$myThreads" |
                Add-Content -Path $Csv
        }
    } elseif (-not $script:activeWx) {
        Write-Host "  [跳过对比] 微信小游戏未运行" -ForegroundColor DarkGray
    } else {
        Write-Host "  [跳过对比] SDK进程未运行" -ForegroundColor DarkGray
    }

    Write-Host ""
}

# ── 主入口 ───────────────────────────────────────────────

if ($Csv) {
    "timestamp,wx_cpu_pct,my_cpu_pct,wx_pss_kb,my_pss_kb,wx_native_kb,my_native_kb,wx_java_kb,my_java_kb,wx_gfx_kb,my_gfx_kb,wx_threads,my_threads" |
        Set-Content -Path $Csv
    Write-Host "CSV => $Csv"
}

Write-Host "微信沙箱 : com.tencent.mm:appbrand*  (自动识别CPU最高的为活跃)"
Write-Host "SDK进程  : $MY_SDK"
Write-Host "采集 $Loop 次，间隔 ${Interval}s"
Write-Host ""

for ($i = 1; $i -le $Loop; $i++) {
    Collect-Snapshot $i
    if ($i -lt $Loop) { Start-Sleep -Seconds $Interval }
}

Write-Host "完成。" -ForegroundColor Cyan
