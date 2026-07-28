#!/usr/bin/env bash
# ============================================================
# 一键部署+真机验证脚本(瘦身工程各阶段共用)
# Location: scripts/deploy-verify.sh
#
# 链路:build .so -> build-aar(debug variant, release .so)
#       -> build demo APK -> install -> 恢复游戏数据 -> 启动 -> 抓日志
#
# 用法:
#   ./scripts/deploy-verify.sh                 # arm64-v8a,完整链路
#   ./scripts/deploy-verify.sh --skip-so       # 跳过 .so 重编(已构建好)
#   ./scripts/deploy-verify.sh --no-uninstall  # 不卸载重装(签名一致时)
# ============================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
EXAMPLES_ROOT="${MIGO_EXAMPLES_ROOT:-$REPO_ROOT/../migo-examples}"
DEMO_DIR="$EXAMPLES_ROOT/android-java"
ADB="${ADB:-$HOME/Android/Sdk/platform-tools/adb}"
PKG="com.minigame.androiddemo"
ABI="arm64-v8a"

SKIP_SO=false
UNINSTALL=true
for a in "$@"; do
    case "$a" in
        --skip-so) SKIP_SO=true ;;
        --no-uninstall) UNINSTALL=false ;;
        arm64-v8a|x86_64) ABI="$a" ;;
    esac
done

info()  { echo -e "\033[0;36m[VERIFY] $1\033[0m"; }
ok()    { echo -e "\033[0;32m[OK] $1\033[0m"; }
err()   { echo -e "\033[0;31m[ERR] $1\033[0m"; }

# 1) build .so (release)
if [[ "$SKIP_SO" == false ]]; then
    info "构建 $ABI release .so"
    bash "$SCRIPT_DIR/build-android-so.sh" "$ABI" release
fi

# 2) build AAR (debug variant 产出 migo-full-debug.aar;.so 已是 release)
info "打包 AAR (debug variant + 现有 release .so)"
bash "$SCRIPT_DIR/build-aar.sh" debug --skip-rust "$ABI"

# 2.5) 把刚建好的 AAR 交给示例仓库
#
# 示例已迁至 minigame-labs/migo-examples,不再从相对路径直接引用本仓库的 dist —
# 它通过自己的 resolver 取产物,本地模式由 MIGO_LOCAL_REPO 指回这里。产物名由
# resolver 决定(migo-<profile>-debug.aar),所以这里不硬编码文件名。
if [[ ! -x "$EXAMPLES_ROOT/scripts/resolve-migo-artifact.sh" ]]; then
    err "找不到示例仓库: $EXAMPLES_ROOT"
    err "克隆 https://github.com/minigame-labs/migo-examples 到该位置,或设 MIGO_EXAMPLES_ROOT"
    exit 1
fi
info "解析 AAR 到示例仓库"
MIGO_LOCAL_REPO="$REPO_ROOT" bash "$EXAMPLES_ROOT/scripts/resolve-migo-artifact.sh" \
    android-aar "$DEMO_DIR/libs/migo.aar"

# 3) build demo APK
info "构建 demo APK"
( cd "$DEMO_DIR" && bash ./gradlew assembleDebug )
APK="$DEMO_DIR/app/build/outputs/apk/debug/app-debug.apk"
[[ -f "$APK" ]] || { err "APK 未生成: $APK"; exit 1; }

# 4) 备份游戏数据(若已安装) -> 卸载重装 -> 恢复
if "$ADB" shell pm list packages 2>/dev/null | grep -q "$PKG"; then
    info "备份游戏数据"
    "$ADB" exec-out run-as "$PKG" tar cf - -C files/migo games > /tmp/migo-games-backup.tar 2>/dev/null || true
fi

if [[ "$UNINSTALL" == true ]]; then
    info "卸载旧 app"
    "$ADB" uninstall "$PKG" 2>/dev/null || true
fi

info "安装 APK"
"$ADB" install -r "$APK" 2>&1 | tail -1

# 启动一次初始化 filesDir,再恢复数据
"$ADB" shell monkey -p "$PKG" -c android.intent.category.LAUNCHER 1 >/dev/null 2>&1 || true
sleep 2
"$ADB" shell am force-stop "$PKG" 2>/dev/null || true

if [[ -f /tmp/migo-games-backup.tar ]]; then
    info "恢复游戏数据"
    "$ADB" shell run-as "$PKG" mkdir -p files/migo 2>/dev/null || true
    "$ADB" shell "run-as $PKG tar xf - -C files/migo" < /tmp/migo-games-backup.tar
fi

# 5) 启动 + 抓日志
info "启动游戏,抓 10s 日志"
"$ADB" logcat -c 2>/dev/null || true
"$ADB" shell am start -n "$PKG/.MainActivity" >/dev/null 2>&1
sleep 10

echo "---- migo / 崩溃相关日志 ----"
"$ADB" logcat -d 2>/dev/null | grep -iE "\[migo\]|MigoDemo|FATAL|AndroidRuntime|libmigo|panic|abort|tombstone|op_load_image" | tail -30

if "$ADB" shell pidof "$PKG" >/dev/null 2>&1; then
    ok "进程存活,验证通过"
else
    err "进程已退出,检查日志"
    exit 1
fi
