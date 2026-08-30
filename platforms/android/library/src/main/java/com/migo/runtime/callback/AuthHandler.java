package com.migo.runtime.callback;

/**
 * Host-provided authentication handler for {@code migo.login}/{@code migo.checkSession}.
 * <p>
 * Register this handler via {@link com.migo.runtime.GameSession#setAuthHandler(AuthHandler)}
 * before the game starts, then bridge to your platform auth SDK.
 *
 * <h2>Without a handler</h2>
 * Every call fails rather than stalling or reporting a signed-in user:
 * {@code migo.login()}, {@code migo.checkSession()}, {@code migo.getUserInfo()}
 * and {@code migo.getPhoneNumber()} all settle with {@code no auth handler}.
 * <p>
 * Failing is the point. The runtime holds no account system and cannot mint a
 * session, so answering success with nobody authenticated would hand content a
 * player identity that exists only inside the game.
 *
 * <p>Contract:
 * <ul>
 *   <li>{@link #login(int, LoginCallback)} must eventually invoke one callback method.</li>
 *   <li>{@link #checkSession(CheckSessionCallback)} must eventually invoke one callback method.</li>
 *   <li>Callbacks may be invoked from any thread.</li>
 * </ul>
 */
public interface AuthHandler {

    /**
     * Perform login and return a one-time code.
     *
     * @param timeoutMs requested timeout from JS options, 0 if not provided
     * @param callback  completion callback
     */
    void login(int timeoutMs, LoginCallback callback);

    /**
     * Check whether current session is still valid.
     *
     * @param callback completion callback
     */
    void checkSession(CheckSessionCallback callback);

    /**
     * Get user profile info.
     * <p>
     * Default behavior is unsupported.
     *
     * @param withCredentials whether sensitive encrypted fields are requested
     * @param lang            preferred language: {@code en}, {@code zh_CN}, {@code zh_TW}
     * @param callback        completion callback
     */
    default void getUserInfo(boolean withCredentials, String lang, UserInfoCallback callback) {
        if (callback != null) {
            callback.onFailure("not supported");
        }
    }

    /**
     * Get phone number one-time token.
     * <p>
     * Default behavior is unsupported.
     *
     * @param isRealtime              whether realtime verification is requested
     * @param phoneNumberNoQuotaToast whether quota toast should be shown when exhausted
     * @param callback                completion callback
     */
    default void getPhoneNumber(boolean isRealtime, boolean phoneNumberNoQuotaToast, PhoneNumberCallback callback) {
        if (callback != null) {
            callback.onFailure("not supported", null);
        }
    }

    interface LoginCallback {
        /**
         * Called when login succeeds.
         *
         * @param code one-time login code
         */
        void onSuccess(String code);

        /**
         * Called when login fails.
         *
         * @param reason failure reason (without API prefix)
         */
        void onFailure(String reason);
    }

    interface CheckSessionCallback {
        /** Called when session is valid. */
        void onSuccess();

        /**
         * Called when session is invalid.
         *
         * @param reason failure reason (without API prefix)
         */
        void onFailure(String reason);
    }

    interface UserInfoCallback {
        /**
         * Called when getUserInfo succeeds.
         *
         * @param result user info payload
         */
        void onSuccess(UserInfoResult result);

        /**
         * Called when getUserInfo fails.
         *
         * @param reason failure reason (without API prefix)
         */
        void onFailure(String reason);
    }

    interface PhoneNumberCallback {
        /**
         * Called when getPhoneNumber succeeds.
         *
         * @param code one-time token code
         */
        void onSuccess(String code);

        /**
         * Called when getPhoneNumber fails.
         *
         * @param reason failure reason (without API prefix)
         * @param errno  optional platform errno
         */
        void onFailure(String reason, Integer errno);
    }

    /** User profile fields mapped to migo.getUserInfo(). */
    final class UserInfo {
        public String nickName = "";
        public String avatarUrl = "";
        public int gender = 0;
        public String country = "";
        public String province = "";
        public String city = "";
        public String language = "zh_CN";
    }

    /** Payload returned by getUserInfo callback. */
    final class UserInfoResult {
        public UserInfo userInfo = new UserInfo();
        public String rawData;
        public String signature;
        public String encryptedData;
        public String iv;
        public String cloudID;
    }
}
