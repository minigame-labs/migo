package com.migo.runtime.callback;

/**
 * Host-provided settings handler.
 * <p>
 * Register via
 * {@link com.migo.runtime.GameSession#setSettingHandler(SettingHandler)} before
 * the game starts, then show your own permission-management UI. The runtime has
 * no settings screen of its own: the standing decisions live with you, seeded
 * and revised through {@link PermissionSink#setScope}, so the screen that edits
 * them has to be yours too.
 *
 * <h2>Without a handler</h2>
 * {@code wx.openSetting()} fails with {@code openSetting:fail not supported} and
 * code {@code -2}. It settles rather than staying silent, because content that
 * was denied a scope is meant to be sent here -- a stalled {@code openSetting()}
 * is a game with no way back.
 *
 * <h2>Contract</h2>
 * <ul>
 *   <li>{@link #openSetting} must eventually settle, exactly once, through the
 *       {@link SettingSink} it is given.</li>
 *   <li>Calls arrive on the runtime's host thread. Do not block: start your UI
 *       and return. The sink is safe to use from any thread.</li>
 *   <li>Report the scopes you show as granted through
 *       {@link PermissionSink#setScope} as well. What the sink carries is what
 *       this one call reports; what {@code setScope} carries is what every
 *       capability check reads.</li>
 * </ul>
 */
public interface SettingHandler {

    /**
     * Show the permission-management UI for this game.
     * <p>
     * Backs {@code wx.openSetting()}. Content typically calls it after a
     * refusal, so the expectation is a screen the user can change a decision on,
     * not a read-only report.
     *
     * @param sink channel to settle this request on
     */
    default void openSetting(SettingSink sink) {
        sink.fail(-2, "openSetting:fail not supported");
    }
}
