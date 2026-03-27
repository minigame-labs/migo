package com.migo.runtime.internal;

import android.app.Activity;
import android.content.Context;
import android.content.Intent;
import android.os.Bundle;

import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * A transparent proxy Activity that handles {@code startActivityForResult} internally,
 * so the host Activity does not need to override {@code onActivityResult}.
 *
 * @hide
 */
public class ResultProxyActivity extends Activity {

    /**
     * Callback interface for receiving activity results.
     */
    public interface ResultCallback {
        void onResult(int requestCode, int resultCode, Intent data);
    }

    private static final ConcurrentHashMap<Integer, PendingRequest> sPendingRequests =
            new ConcurrentHashMap<>();
    private static final AtomicInteger sRequestCodeCounter = new AtomicInteger(10000);

    private int assignedRequestCode = -1;
    private boolean launched = false;

    private static final class PendingRequest {
        final Intent intent;
        final int requestCode;
        final ResultCallback callback;

        PendingRequest(Intent intent, int requestCode, ResultCallback callback) {
            this.intent = intent;
            this.requestCode = requestCode;
            this.callback = callback;
        }
    }

    /**
     * Launch a transparent proxy Activity that will call {@code startActivityForResult}
     * on behalf of the caller and deliver the result via {@code callback}.
     *
     * @param context       A Context (typically an Activity) used to start this proxy
     * @param targetIntent  The Intent to forward to {@code startActivityForResult}
     * @param requestCode   The request code (used for callback identification only)
     * @param callback      Receives the result when the target Activity finishes
     */
    public static void launch(Context context, Intent targetIntent, int requestCode,
                              ResultCallback callback) {
        int uniqueCode = sRequestCodeCounter.getAndIncrement();
        sPendingRequests.put(uniqueCode, new PendingRequest(targetIntent, requestCode, callback));
        Intent proxy = new Intent(context, ResultProxyActivity.class);
        proxy.putExtra("_proxy_request_code", uniqueCode);
        if (!(context instanceof Activity)) {
            proxy.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        }
        context.startActivity(proxy);
    }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        overridePendingTransition(0, 0);

        assignedRequestCode = getIntent().getIntExtra("_proxy_request_code", -1);
        PendingRequest req = assignedRequestCode >= 0
                ? sPendingRequests.get(assignedRequestCode) : null;
        if (req == null) {
            // Process was killed and recreated — static state lost, nothing to do
            finish();
            overridePendingTransition(0, 0);
            return;
        }

        launched = true;
        startActivityForResult(req.intent, req.requestCode);
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);

        PendingRequest req = assignedRequestCode >= 0
                ? sPendingRequests.remove(assignedRequestCode) : null;

        if (req != null && req.callback != null) {
            req.callback.onResult(requestCode, resultCode, data);
        }

        finish();
        overridePendingTransition(0, 0);
    }

    @Override
    protected void onDestroy() {
        super.onDestroy();
        // If destroyed without receiving a result (e.g. user pressed back on this activity
        // before the target launched), clean up pending state
        if (!isFinishing() || !launched) {
            if (assignedRequestCode >= 0) {
                sPendingRequests.remove(assignedRequestCode);
            }
        }
    }
}
