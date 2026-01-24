package com.minigame.host.internal;

import android.app.Activity;
import android.content.Context;
import android.content.Intent;

import com.minigame.host.internal.bluetooth.BluetoothSettingActivity;
import com.minigame.host.internal.device.DeviceInfoHelper;
import com.minigame.host.internal.io.ZipHelper;
import com.minigame.host.internal.runtime.HostRuntimeRegistry;
import com.minigame.host.internal.system.PermissionHelper;
import com.minigame.host.internal.system.SystemSettingHelper;
import com.minigame.host.internal.window.WindowInfoHelper;

class NativeExports {
    static void unzip(int hostId, int requestId, String zipPath, String destDir) {
        ZipHelper.unzip(hostId, requestId, zipPath, destDir);
    }

    static String getCacheDirPath() {
        return AppContext.get().getCacheDir().getAbsolutePath();
    }

    static void openSystemBluetoothSetting(int hostId) {
        Context ctx = AppContext.get();

        Intent intent = new Intent(ctx, BluetoothSettingActivity.class);
        intent.addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP);
        intent.putExtra("com.minigame.host.EXTRA_HOST_ID", hostId);

        if (!(ctx instanceof Activity)) {
            intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        }
        ctx.startActivity(intent);
    }

    static byte[] getWindowInfoBytes(int hostId) {
        Activity activity = HostRuntimeRegistry.getActivity(hostId);
        if (activity == null) return null;
        return WindowInfoHelper.getAsBytes(activity);
    }

    static byte[] getSystemSettingInfoBytes() {
        return SystemSettingHelper.getAsBytes(AppContext.get());
    }

    static String getDeviceInfoJson() {
        return DeviceInfoHelper.getAsJson(AppContext.get());
    }

    static String getAppAuthorizationSettingJson() {
        return PermissionHelper.getAsJson(AppContext.get());
    }
}
