package com.migo.runtime.callback;

import java.util.Map;

/**
 * The channel a {@link SettingHandler} settles {@code wx.openSetting()} on.
 * <p>
 * Exactly one of these methods takes effect. The first call settles the
 * request; later calls do nothing, so a handler that reports both a dismissal
 * and a failure cannot deliver two results to content.
 *
 * <h2>Threading</h2>
 * Every method is safe to call from any thread. That is the point: settling
 * usually means an Activity result or a dialog callback, which arrives on
 * whatever thread the platform chose.
 *
 * <p>Calls for a session that has ended are ignored, so you do not have to race
 * teardown to stay correct.
 */
public interface SettingSink {

    /**
     * Report that the settings UI was shown and the user is finished with it.
     * <p>
     * {@code authSetting} is what content reads back as {@code res.authSetting},
     * keyed by wx scope name ({@code "scope.camera"}). The runtime does not store
     * it: the standing decision every capability call is checked against is the
     * one you set through {@link PermissionSink#setScope}, and
     * {@code wx.getSetting()} reads that rather than this. Report a grant here
     * that you did not also record there and the game acts once, then finds the
     * capability refused -- which reads as a bug in your app rather than as a
     * permission model.
     * <p>
     * A null or empty map settles with an empty {@code authSetting} object, never
     * with the field absent: wx always carries it on a success, and content reads
     * {@code res.authSetting[scope]} without checking. Report the scopes as they
     * stand -- there is no way to say "the user changed nothing" separately from
     * "nothing is granted", so do not rely on one being distinguishable from the
     * other here. The standing decision is the one in {@link PermissionSink#setScope}.
     *
     * @param authSetting the scopes as they now stand, or null for none
     */
    void settleOpened(Map<String, Boolean> authSetting);

    /**
     * Settle the request as failed.
     *
     * @param errCode SDK-specific error code, or -1 if there is none
     * @param errMsg  the message content receives verbatim as {@code errMsg},
     *                conventionally {@code "openSetting:fail <reason>"}
     */
    void fail(int errCode, String errMsg);
}
