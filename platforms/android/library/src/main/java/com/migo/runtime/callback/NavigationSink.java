package com.migo.runtime.callback;

/**
 * The channel a {@link NavigationHandler} settles
 * {@code wx.navigateToMiniProgram()} on.
 * <p>
 * Exactly one of these methods takes effect. The first call settles the
 * request; later calls do nothing, so a host that reports both an arrival and a
 * cancellation cannot deliver two results to content.
 *
 * <h2>Threading</h2>
 * Every method is safe to call from any thread, which is what you need to start
 * an Activity and answer from its result callback.
 *
 * <p>Calls for a session that has ended are ignored, so you do not have to race
 * teardown to stay correct.
 *
 * <p>{@link NavigationHandler#openCustomerServiceConversation} has no sink: the
 * runtime settles it from that method's own return, for the reason recorded
 * there.
 */
public interface NavigationSink {

    /**
     * Report that the destination was reached.
     */
    void settleNavigated();

    /**
     * Settle the request as failed.
     *
     * @param errCode SDK-specific error code, or -1 if there is none
     * @param errMsg  the message content receives verbatim as {@code errMsg},
     *                conventionally {@code "navigateToMiniProgram:fail <reason>"}
     */
    void fail(int errCode, String errMsg);
}
