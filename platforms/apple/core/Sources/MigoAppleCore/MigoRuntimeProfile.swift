/// Which runtime lane a Session runs in.
///
/// The value is frozen before any content code executes and never changes for
/// the life of the Session. There is no lossless switch at runtime: the lanes
/// do not share a JavaScript heap, a renderer, or resource ownership, so what
/// reads as "falling back" is either a choice made before user code ran, or a
/// reload under a new runtime generation.
public enum MigoRuntimeProfile: UInt32, Sendable {
    /// WebKit runs the JavaScript and WebKit renders. The compatibility and
    /// safety baseline, and the right answer -- not a consolation -- for
    /// content that leans on synchronous GPU queries, Web Audio, or browser
    /// module semantics, and for remote content whose command stream cannot be
    /// validated ahead of time.
    case iosWebKitFull = 1

    /// Content JavaScript and WebAssembly run in a Worker inside WebKit's
    /// WebContent process, where the system grants JIT. One bounded binary
    /// frame packet per frame crosses into this process, which renders it.
    case iosPerformancePlus = 2

    /// In-process V8 with JIT. macOS only: there is no JIT restriction to work
    /// around, so a second process would buy crash isolation and nothing else.
    case macosV8Native = 3

    /// WebKit end to end on macOS. Selected for untrusted remote content, and
    /// when the JIT entitlement is absent or the signature does not validate.
    case macosWebKitFull = 4
}

/// Stable identifiers for dynamic memory-policy inputs.
///
/// The observations represented by these identifiers are runtime values. They
/// are deliberately not represented here as byte constants, and none is a
/// jetsam limit guarantee.
public enum MigoMemoryPolicyField: String, Sendable {
    case contentCap = "content_cap"
    case hostCap = "host_cap"
    case measuredDeviceSafeCap = "measured_device_safe_cap"
    case availableMemoryHeadroom = "available_memory_headroom"
    case emergencyReserve = "emergency_reserve"
    case reservationBeforeAlloc = "reservation_before_alloc"
    case requeryBeforeLargeReservation = "requery_before_large_reservation"
}

/// The reason a profile was chosen, carried alongside every decision.
///
/// A profile without a reason is unactionable in production: the question that
/// actually gets asked is never "which lane" but "why did *this* device, on
/// *this* content, end up there", and by then the inputs are gone.
public enum MigoProfileReason: String, Sendable {
    case hostRequested
    case contentManifestRequires
    case osBelowLaneMinimum
    case deviceTierBelowLaneMinimum
    case capabilityProbeFailed
    case denylisted
    case remoteKillSwitch
    case initializationFailed
    case previousGenerationLost
}
