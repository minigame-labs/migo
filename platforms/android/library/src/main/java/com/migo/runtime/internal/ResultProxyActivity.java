package com.migo.runtime.internal;

import android.app.Activity;
import android.content.Context;
import android.content.Intent;
import android.os.Bundle;

/**
 * A transparent proxy Activity that handles {@code startActivityForResult} internally,
 * so the host Activity does not need to override {@code onActivityResult}.
 *
 * <p>The request itself lives in {@link ProxyRequestTable} until this Activity
 * is created; from then on this instance owns it and answers it exactly once,
 * whether the answer is a result, a cancellation, or the target never starting.
 * Activity lifecycle callbacks all arrive on the main thread, which is what
 * makes a plain field enough to hold that ownership.
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

    /**
     * Names the request in the Intent. Distinct from the {@code int} key this
     * replaced, so an Intent left over from an older build cannot be read as a
     * token that names somebody else's request.
     */
    private static final String EXTRA_TOKEN = "_proxy_request_token";

    private static final ProxyRequestTable<PendingRequest> sPending = new ProxyRequestTable<>();

    /** The request this instance owes a result to; null once it has answered. */
    private PendingRequest owned;

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
     * <p>If the proxy cannot be started, the recorded request is removed before
     * the failure propagates: the caller's own error path then answers it, and
     * a token that nothing will ever arrive for is not left behind. The caller
     * must not also treat the throw as a delivery — it is one or the other.
     *
     * @param context       A Context (typically an Activity) used to start this proxy
     * @param targetIntent  The Intent to forward to {@code startActivityForResult}
     * @param requestCode   The request code (used for callback identification only)
     * @param callback      Receives the result when the target Activity finishes
     */
    public static void launch(Context context, Intent targetIntent, int requestCode,
                              ResultCallback callback) {
        long token = sPending.register(new PendingRequest(targetIntent, requestCode, callback));
        Intent proxy = new Intent(context, ResultProxyActivity.class);
        proxy.putExtra(EXTRA_TOKEN, token);
        if (!(context instanceof Activity)) {
            proxy.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        }
        try {
            context.startActivity(proxy);
        } catch (RuntimeException notStarted) {
            sPending.take(token);
            throw notStarted;
        }
    }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        overridePendingTransition(0, 0);

        Intent intent = getIntent();
        PendingRequest request = intent != null && intent.hasExtra(EXTRA_TOKEN)
                ? sPending.take(intent.getLongExtra(EXTRA_TOKEN, 0L))
                : null;
        if (request == null) {
            // Process was killed and recreated — the table naming this request
            // died with it, so there is nobody left to answer.
            finish();
            overridePendingTransition(0, 0);
            return;
        }

        owned = request;
        try {
            startActivityForResult(request.intent, request.requestCode);
        } catch (RuntimeException notStarted) {
            deliver(request.requestCode, Activity.RESULT_CANCELED, null);
            finish();
            overridePendingTransition(0, 0);
        }
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);

        deliver(requestCode, resultCode, data);

        finish();
        overridePendingTransition(0, 0);
    }

    @Override
    protected void onDestroy() {
        super.onDestroy();
        // Destroyed without receiving a result (e.g. target Activity crashed, user pressed
        // back before the target launched): the request is still owed an answer.
        PendingRequest request = owned;
        if (request != null) {
            deliver(request.requestCode, Activity.RESULT_CANCELED, null);
        }
    }

    /** Answers the owned request, if it has not been answered already. */
    private void deliver(int requestCode, int resultCode, Intent data) {
        PendingRequest request = owned;
        if (request == null) return;
        owned = null;
        if (request.callback != null) {
            request.callback.onResult(requestCode, resultCode, data);
        }
    }
}
