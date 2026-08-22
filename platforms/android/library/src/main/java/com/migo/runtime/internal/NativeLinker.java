package com.migo.runtime.internal;

/**
 * The two ways a JNI library can enter the process, behind an interface.
 * <p>
 * The load decision is a state machine with several failure modes that only
 * appear on a user's device -- an absent file, a truncated download, a slice
 * built for the other ABI. Making the linker injectable is what lets that
 * state machine be exercised on a host JVM instead of being discovered in the
 * field, which matters more here than usual: packaging the engine outside the
 * APK converts a class of build-time failure into a runtime one.
 *
 * @hide
 */
public interface NativeLinker {

    /** {@code System.loadLibrary} -- resolves through the APK's JNI directories. */
    void loadLibrary(String name);

    /** {@code System.load} -- an absolute path the host delivered. */
    void load(String absolutePath);

    /** The real linker. */
    NativeLinker SYSTEM = new NativeLinker() {
        @Override
        public void loadLibrary(String name) {
            System.loadLibrary(name);
        }

        @Override
        public void load(String absolutePath) {
            System.load(absolutePath);
        }

        @Override
        public String toString() {
            return "NativeLinker.SYSTEM";
        }
    };
}
