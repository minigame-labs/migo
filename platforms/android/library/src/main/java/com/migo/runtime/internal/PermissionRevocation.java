package com.migo.runtime.internal;

import java.util.function.BooleanSupplier;

/** Rejects late updates and routes native-requested teardown by scope. */
final class PermissionRevocation {
    interface ResourceTeardown {
        void destroyCamera(int sessionId);
        void destroyRecorder(int sessionId);
        void destroyBluetooth(int sessionId);
    }

    private PermissionRevocation() {}

    static boolean update(
            String scope,
            BooleanSupplier sessionTerminated,
            BooleanSupplier updateNative) {
        if (scope == null || scope.isEmpty() || sessionTerminated.getAsBoolean()) return false;
        return updateNative.getAsBoolean();
    }

    static void tearDown(
            int sessionId,
            String scope,
            ResourceTeardown resources,
            Runnable terminateSession) {
        try {
            switch (scope) {
                case "scope.camera":
                    resources.destroyCamera(sessionId);
                    break;
                case "scope.record":
                    resources.destroyRecorder(sessionId);
                    break;
                case "scope.bluetooth":
                    resources.destroyBluetooth(sessionId);
                    break;
                default:
                    break;
            }
        } catch (RuntimeException cleanupFailure) {
            try {
                terminateSession.run();
            } catch (RuntimeException terminationFailure) {
                cleanupFailure.addSuppressed(terminationFailure);
            }
            throw cleanupFailure;
        }
    }
}
