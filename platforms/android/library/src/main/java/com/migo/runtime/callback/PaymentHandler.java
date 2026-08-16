package com.migo.runtime.callback;

/**
 * Host-provided payment handler.
 * <p>
 * Register via
 * {@link com.migo.runtime.GameSession#setPaymentHandler(PaymentHandler)} before
 * the game starts, then bridge each call to your billing integration (Google
 * Play Billing, a carrier channel, your own wallet). The runtime links no
 * payment SDK, holds no merchant credentials, and never decides that a purchase
 * succeeded.
 *
 * <h2>Without a handler</h2>
 * {@link #isMidasPaymentSupported} reports {@code false}, so
 * {@code migo.checkIsSupportMidasPayment()} answers
 * {@code {allow_pay: false}} and well-behaved content never opens a store it
 * cannot transact in. Content that asks anyway fails with
 * {@code requestMidasPayment:fail not supported} and code {@code -2}.
 *
 * <h2>Contract</h2>
 * <ul>
 *   <li>{@link #requestMidasPayment} and
 *       {@link #requestMidasPaymentGameItem} must eventually settle, exactly
 *       once, through the {@link PaymentSink} they are given.</li>
 *   <li>{@link #isMidasPaymentSupported} is answered synchronously on the
 *       runtime's host thread, because content asks it synchronously. Return a
 *       value you already hold; do not query a network from it.</li>
 *   <li>Calls arrive on the runtime's host thread. Do not block: present your
 *       payment UI and return. The sink is safe to use from any thread.</li>
 *   <li>Reporting {@code true} from {@link #isMidasPaymentSupported} while
 *       leaving the request methods unimplemented advertises a store that
 *       refuses every purchase.</li>
 * </ul>
 */
public interface PaymentHandler {

    /**
     * Whether this device and user can transact at all.
     * <p>
     * Backs {@code migo.checkIsSupportMidasPayment()}, which content calls before
     * showing a store. It is the answer to "is there a payment channel here",
     * not "will this particular purchase go through".
     *
     * @return whether purchases are possible
     */
    default boolean isMidasPaymentSupported() {
        return false;
    }

    /**
     * Sell game currency.
     *
     * @param request what content asked to charge for
     * @param sink    channel to settle this purchase on
     */
    default void requestMidasPayment(PaymentRequest request, PaymentSink sink) {
        sink.fail(-2, "requestMidasPayment:fail not supported");
    }

    /**
     * Sell a specific game item, priced and signed by the game's own server.
     * <p>
     * Distinct from {@link #requestMidasPayment} in who set the price: here the
     * game server did, and the signature is what attests to it. A host that
     * charges without checking the signature against its own server is trusting
     * a price content chose.
     *
     * @param request the signed order content asked to charge for
     * @param sink    channel to settle this purchase on
     */
    default void requestMidasPaymentGameItem(GameItemPaymentRequest request, PaymentSink sink) {
        sink.fail(-2, "requestMidasPaymentGameItem:fail not supported");
    }

    /** What content asked to charge for. */
    final class PaymentRequest {
        /** platform payment mode, {@code "game"} unless content chose otherwise. */
        public final String mode;
        /** platform environment selector: {@code 0} for production, {@code 1} for sandbox. */
        public final int env;
        /** The publisher's offer id, as content supplied it. */
        public final String offerId;
        /** ISO currency code, {@code "CNY"} unless content chose otherwise. */
        public final String currencyType;
        /** platform selector, empty when content set none. */
        public final String platform;
        /** How much game currency to buy; {@code 0} when content set none. */
        public final int buyQuantity;
        /** Game zone id, {@code 1} unless content chose otherwise. */
        public final int zoneId;
        /** The game's own order number, empty when content set none. */
        public final String outTradeNo;

        public PaymentRequest(String mode, int env, String offerId, String currencyType,
                              String platform, int buyQuantity, int zoneId, String outTradeNo) {
            this.mode = mode;
            this.env = env;
            this.offerId = offerId;
            this.currencyType = currencyType;
            this.platform = platform;
            this.buyQuantity = buyQuantity;
            this.zoneId = zoneId;
            this.outTradeNo = outTradeNo;
        }
    }

    /** The signed order content asked to charge for. */
    final class GameItemPaymentRequest {
        /** The order the game's server signed, verbatim; verify it, do not parse it for price. */
        public final String signData;
        /** Payment signature over {@link #signData}. */
        public final String paySig;
        /** Session signature over {@link #signData}. */
        public final String signature;

        public GameItemPaymentRequest(String signData, String paySig, String signature) {
            this.signData = signData;
            this.paySig = paySig;
            this.signature = signature;
        }
    }
}
