package com.migo.runtime.internal;

import com.migo.runtime.callback.NavigationHandler;
import com.migo.runtime.callback.NavigationSink;
import com.migo.runtime.callback.PaymentHandler;
import com.migo.runtime.callback.PaymentSink;
import com.migo.runtime.callback.SettingSink;
import com.migo.runtime.callback.ShareHandler;
import com.migo.runtime.callback.ShareSink;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.function.BooleanSupplier;

/**
 * The commercial host callbacks' half of the boundary that has no {@code android.*} in it.
 *
 * <p>Two jobs, both deliberately here rather than in {@code NativeExports}. Parsing turns
 * the request JSON the engine sends into the immutable typed objects the published handler
 * interfaces take, so no raw JSON reaches an embedder. Settling turns a handler's answer
 * back into the result JSON the runtime routes, once per request.
 *
 * <p>{@code NativeExports} holds {@code android.os.Handler} statics, so initialising it
 * needs a device; this class depends on nothing but {@code org.json}, which is what keeps
 * the parse and the settlement reachable from a host-JVM unit test. That is the same split
 * {@code AdOp} makes for the ad bridge, and for the same reason: the decisions worth
 * testing must not live in the untestable half.
 */
final class HostDelegation {

    private HostDelegation() {}

    /** Where a settled result goes: one of the {@code NativeMethods.on*Result} forwarders. */
    interface ResultChannel {
        void deliver(int sessionId, String resultJson);
    }

    /**
     * One deferred request, settled at most once.
     *
     * <p>A second settlement would emit a second result for one {@code requestId}, and the
     * runtime routes each result to content: a payment that reported both a completion and
     * a cancellation would fire content's {@code success} and its {@code fail}. Hosts hit
     * this by accident -- a payment SDK that calls both its completion and its dismissal
     * callback is ordinary -- so the guarantee is enforced here rather than documented as a
     * rule to follow. Same {@code compareAndSet} claim the auth and subpackage callbacks
     * use, hoisted out of them so there is one implementation of "answers once".
     */
    static final class Settlement {
        private final int sessionId;
        private final int requestId;
        private final ResultChannel channel;
        private final BooleanSupplier sessionTerminated;
        private final AtomicBoolean settled = new AtomicBoolean(false);

        Settlement(
                int sessionId,
                int requestId,
                ResultChannel channel,
                BooleanSupplier sessionTerminated) {
            this.sessionId = sessionId;
            this.requestId = requestId;
            this.channel = channel;
            this.sessionTerminated = sessionTerminated;
        }

        /**
         * Settle as successful.
         *
         * @param payload fields content reads off the result, or null for none
         */
        void succeed(Map<String, Object> payload) {
            if (!claim()) return;
            String resultJson;
            try {
                JSONObject result = new JSONObject();
                CallbackCorrelation.stamp(result, requestId);
                if (payload != null) {
                    for (Map.Entry<String, Object> field : payload.entrySet()) {
                        result.put(field.getKey(), field.getValue());
                    }
                }
                resultJson = result.toString();
            } catch (JSONException unserialisable) {
                // A success that cannot be serialised must not read as a success with its
                // fields missing: content would act on a reply it never got.
                resultJson = failure(requestId, -1, "result serialisation failed");
            }
            deliver(resultJson);
        }

        /** Settle as failed, with the message content receives verbatim. */
        void fail(int errCode, String errMsg) {
            if (!claim()) return;
            deliver(failure(requestId, errCode, errMsg));
        }

        /** Whether this request has already been answered. */
        boolean isSettled() {
            return settled.get();
        }

        private boolean claim() {
            return settled.compareAndSet(false, true);
        }

        private void deliver(String resultJson) {
            if (sessionTerminated.getAsBoolean()) return;
            channel.deliver(sessionId, resultJson);
        }
    }

    /**
     * The failure shape the runtime's deferred-result plumbing reads: {@code error} rejects
     * the pending call and {@code errCode} is copied onto what content sees.
     */
    static String failure(int requestId, int errCode, String errMsg) {
        String reason = errMsg != null ? errMsg : "unknown error";
        try {
            JSONObject result = new JSONObject();
            CallbackCorrelation.stamp(result, requestId);
            result.put("error", reason);
            result.put("errCode", errCode);
            return result.toString();
        } catch (JSONException impossible) {
            // Only for a null key or a non-finite number, neither of which occurs above.
            return "{\"error\":\"result serialisation failed\",\"errCode\":" + errCode + "}";
        }
    }

    // ==================== Sinks ====================

    static SettingSink settingSink(Settlement settlement) {
        return new SettingSink() {
            @Override
            public void settleOpened(Map<String, Boolean> authSetting) {
                JSONObject scopes = new JSONObject();
                if (authSetting != null) {
                    for (Map.Entry<String, Boolean> scope : authSetting.entrySet()) {
                        if (scope.getKey() == null || scope.getValue() == null) continue;
                        try {
                            scopes.put(scope.getKey(), scope.getValue().booleanValue());
                        } catch (JSONException impossible) {
                            // Thrown only for a null key, filtered above.
                        }
                    }
                }
                // Emitted even when empty. "Nothing is granted" is an answer, and
                // content following the common mini-game platform's idiom reads res.authSetting off a success
                // without checking that it exists -- omitting the field turns that
                // into a TypeError rather than an empty object.
                settlement.succeed(Collections.singletonMap("authSetting", (Object) scopes));
            }

            @Override
            public void fail(int errCode, String errMsg) {
                settlement.fail(errCode, errMsg);
            }
        };
    }

    static ShareSink shareSink(Settlement settlement) {
        return new ShareSink() {
            @Override
            public void settleShared() {
                settlement.succeed(null);
            }

            @Override
            public void fail(int errCode, String errMsg) {
                settlement.fail(errCode, errMsg);
            }
        };
    }

    static NavigationSink navigationSink(Settlement settlement) {
        return new NavigationSink() {
            @Override
            public void settleNavigated() {
                settlement.succeed(null);
            }

            @Override
            public void fail(int errCode, String errMsg) {
                settlement.fail(errCode, errMsg);
            }
        };
    }

    static PaymentSink paymentSink(Settlement settlement) {
        return new PaymentSink() {
            @Override
            public void settlePaid() {
                settlement.succeed(null);
            }

            @Override
            public void fail(int errCode, String errMsg) {
                settlement.fail(errCode, errMsg);
            }
        };
    }

    // ==================== Request parsing ====================

    /** Request options, or an empty object when the engine sent nothing readable. */
    static JSONObject options(String optionsJson) {
        if (optionsJson == null || optionsJson.isEmpty()) return new JSONObject();
        try {
            return new JSONObject(optionsJson);
        } catch (JSONException malformed) {
            // A malformed request still owes content an answer, so it is parsed as empty
            // rather than dropped -- the settlement below is what content is waiting for.
            return new JSONObject();
        }
    }

    static ShareHandler.ShareRequest shareRequest(JSONObject options) {
        return new ShareHandler.ShareRequest(
                options.optString("title", ""),
                options.optString("imageUrl", ""),
                options.optString("query", ""),
                options.optString("imageUrlId", ""));
    }

    static NavigationHandler.NavigateRequest navigateRequest(JSONObject options) {
        return new NavigationHandler.NavigateRequest(
                options.optString("appId", ""),
                options.optString("path", ""),
                immutableMap(options.optJSONObject("extraData")),
                options.optString("envVersion", "release"));
    }

    static NavigationHandler.CustomerServiceRequest customerServiceRequest(JSONObject options) {
        return new NavigationHandler.CustomerServiceRequest(
                options.optString("sessionFrom", ""),
                options.optBoolean("showMessageCard", false),
                options.optString("sendMessageTitle", ""),
                options.optString("sendMessagePath", ""),
                options.optString("sendMessageImg", ""));
    }

    static PaymentHandler.PaymentRequest paymentRequest(JSONObject options) {
        return new PaymentHandler.PaymentRequest(
                options.optString("mode", "game"),
                options.optInt("env", 0),
                options.optString("offerId", ""),
                options.optString("currencyType", "CNY"),
                options.optString("platform", ""),
                options.optInt("buyQuantity", 0),
                options.optInt("zoneId", 1),
                options.optString("outTradeNo", ""));
    }

    static PaymentHandler.GameItemPaymentRequest gameItemPaymentRequest(JSONObject options) {
        return new PaymentHandler.GameItemPaymentRequest(
                options.optString("signData", ""),
                options.optString("paySig", ""),
                options.optString("signature", ""));
    }

    /**
     * A JSON tree as plain immutable collections.
     *
     * <p>{@code extraData} is content's own opaque payload, so it is handed over as
     * {@code Map}/{@code List}/{@code String}/{@code Number}/{@code Boolean} rather than as
     * {@code JSONObject}: a published interface that took an {@code org.json} type would
     * make the SDK's choice of JSON library part of its API, and on Android that type is
     * the platform's rather than the one this module compiles against. Unmodifiable because
     * a host is not the owner of what content sent.
     */
    private static Map<String, Object> immutableMap(JSONObject source) {
        if (source == null || source.length() == 0) return Collections.emptyMap();
        Map<String, Object> out = new LinkedHashMap<>();
        for (java.util.Iterator<String> keys = source.keys(); keys.hasNext(); ) {
            String key = keys.next();
            out.put(key, immutableValue(source.opt(key)));
        }
        return Collections.unmodifiableMap(out);
    }

    private static List<Object> immutableList(JSONArray source) {
        if (source == null || source.length() == 0) return Collections.emptyList();
        List<Object> out = new ArrayList<>(source.length());
        for (int index = 0; index < source.length(); index++) {
            out.add(immutableValue(source.opt(index)));
        }
        return Collections.unmodifiableList(out);
    }

    private static Object immutableValue(Object value) {
        if (value == null || value == JSONObject.NULL) return null;
        if (value instanceof JSONObject) return immutableMap((JSONObject) value);
        if (value instanceof JSONArray) return immutableList((JSONArray) value);
        return value;
    }
}
