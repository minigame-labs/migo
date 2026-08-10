package com.migo.runtime.callback;

/**
 * Host-provided share handler.
 * <p>
 * Register via
 * {@link com.migo.runtime.GameSession#setShareHandler(ShareHandler)} before the
 * game starts, then bridge each call to your own share surface -- a system
 * chooser, your app's friend picker, a social SDK. The runtime links no share
 * SDK and holds no social graph.
 *
 * <h2>Without a handler</h2>
 * {@code wx.shareAppMessage()} fails with
 * {@code shareAppMessage:fail not supported} and code {@code -2}. It settles
 * rather than staying silent: content commonly awaits the share before resuming,
 * so a dropped request is a paused game.
 *
 * <h2>Contract</h2>
 * <ul>
 *   <li>{@link #shareAppMessage} must eventually settle, exactly once, through
 *       the {@link ShareSink} it is given.</li>
 *   <li>Calls arrive on the runtime's host thread. Do not block: present your
 *       share surface and return. The sink is safe to use from any thread.</li>
 *   <li>The game already had its say: {@code wx.onShareAppMessage} listeners ran
 *       before this call and their overrides are folded into the request. Treat
 *       what arrives as the final content, not a default to re-derive.</li>
 * </ul>
 */
public interface ShareHandler {

    /**
     * Present the share flow for one {@code wx.shareAppMessage()} call.
     *
     * @param request what content asked to share
     * @param sink    channel to settle this request on
     */
    default void shareAppMessage(ShareRequest request, ShareSink sink) {
        sink.fail(-2, "shareAppMessage:fail not supported");
    }

    /** What content asked to share. */
    final class ShareRequest {
        /** Share title, empty when content set none. */
        public final String title;
        /** Share image URL, empty when content set none. */
        public final String imageUrl;
        /**
         * Query string the launched game should receive, empty when content set
         * none. wx spells it {@code a=1&b=2}, without a leading {@code ?}.
         */
        public final String query;
        /**
         * wx-platform image id, empty when content set none. Meaningful only to
         * a host that resolves ids against WeChat; ignore it otherwise.
         */
        public final String imageUrlId;

        public ShareRequest(String title, String imageUrl, String query, String imageUrlId) {
            this.title = title;
            this.imageUrl = imageUrl;
            this.query = query;
            this.imageUrlId = imageUrlId;
        }
    }
}
