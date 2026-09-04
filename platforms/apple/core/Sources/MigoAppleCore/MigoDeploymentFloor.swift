/// The deployment floors, derived from contracts/apple/deployment-floor.json.
///
/// These are mirrored here, and checked against the contract by
/// scripts/test-apple-deployment-floor-contract.sh, because runtime code needs
/// to compare against them and `#available` cannot be given a variable.
///
/// The distinction the contract insists on holds here too: `iOS` is the lowest
/// system the binary loads on, and `performancePlusMinimumIOS` is the lowest
/// system on which that one lane is eligible. A device between the two is not
/// unsupported; it runs `MigoRuntimeProfile.iosWebKitFull`.
public enum MigoDeploymentFloor {
    public static let iOS = (major: 15, minor: 0)
    public static let macOS = (major: 11, minor: 0)

    /// WebKit gated SharedArrayBuffer behind crossOriginIsolated at 15.2, and
    /// the Worker's synchronous barrier is built on it. Below this the lane is
    /// not slower, it is structurally impossible -- so the check is a hard gate
    /// rather than a quality tier.
    public static let performancePlusMinimumIOS = (major: 15, minor: 2)
}
