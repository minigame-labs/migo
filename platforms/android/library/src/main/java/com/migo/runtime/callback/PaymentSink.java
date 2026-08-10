package com.migo.runtime.callback;

/**
 * The channel a {@link PaymentHandler} settles one purchase on.
 * <p>
 * Exactly one of these methods takes effect. The first call settles the
 * request; later calls do nothing. This matters more here than anywhere else in
 * this package: two settlements for one purchase would deliver two results to
 * content, and content that credits an item in its {@code success} callback
 * would credit it twice.
 *
 * <h2>Threading</h2>
 * Every method is safe to call from any thread, which is what you need to show a
 * payment dialog and answer on whatever thread your payment SDK reports on.
 *
 * <p>Calls for a session that has ended are ignored, so you do not have to race
 * teardown to stay correct.
 */
public interface PaymentSink {

    /**
     * Report that the purchase completed.
     * <p>
     * Report your payment provider's own verdict and nothing else. Reporting a
     * completion the provider did not confirm hands out paid items nobody paid
     * for -- the same defect as an ad bridge that mints its own rewards, and
     * with a receipt trail that says the publisher is at fault.
     */
    void settlePaid();

    /**
     * Settle the request as failed, which includes the user cancelling.
     * <p>
     * A cancellation is a failure here, not a quiet drop: content is awaiting
     * this call and a purchase flow that reports nothing leaves the player
     * looking at a spinner.
     *
     * @param errCode provider-specific error code, or -1 if there is none
     * @param errMsg  the message content receives verbatim as {@code errMsg},
     *                conventionally {@code "requestMidasPayment:fail <reason>"}
     */
    void fail(int errCode, String errMsg);
}
