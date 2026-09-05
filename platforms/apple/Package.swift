// swift-tools-version: 5.9
//
// GENERATED PLATFORM FLOOR. The two `.iOS`/`.macOS` values below are derived
// from contracts/apple/deployment-floor.json and are checked against it by
// scripts/test-apple-deployment-floor-contract.sh. Change the contract, not
// this line: a hand-edited copy here is exactly the drift the gate exists to
// catch, and it drifts silently because a wrong-but-valid version still builds.

import PackageDescription

let package = Package(
    name: "Migo",
    platforms: [
        .iOS(.v15),
        .macOS(.v11),
    ],
    products: [
        // Three product targets, with Performance+ conditional on the G0
        // real-device selection and the release gates.
        //
        // Keep the products separate so the compatibility baseline does not
        // link ANGLE, Skia, or a renderer it will never call. The Performance+
        // no-V8 claim belongs to the future release gate: this skeleton and
        // product baseline have not proved it, and the current dependency
        // chain may still link V8. An umbrella product would obscure that gate.
        .library(name: "MigoAppleWebKit", targets: ["MigoAppleWebKit"]),
        .library(name: "MigoApplePerformancePlus", targets: ["MigoApplePerformancePlus"]),
        .library(name: "MigoMacV8", targets: ["MigoMacV8"]),
    ],
    // The engine-free half, split out so it can be compiled and tested without
    // the xcframework below. See core/Package.swift for why that mattered: with
    // one package, a missing build output meant no Swift here was ever compiled
    // at all. Path dependency rather than a duplicated target -- two copies of
    // the deployment floors is the drift the floor contract exists to catch.
    dependencies: [
        .package(path: "core"),
    ],
    targets: [
        // The engine, built by scripts/build-apple-sdk.sh.
        //
        // The xcframework carries the C ABI headers and its own modulemap, so
        // there is no separate header target: a vendored second copy of
        // include/migo would be one more thing to drift out of step with the
        // library it describes.
        //
        // Path-based during development and rewritten to a url+checksum form at
        // release time, the way the Android AAR and the Linux SDK already ship.
        // `swift build` fails here until the script has run once; that is the
        // intended failure -- the alternative is unsafeFlags, which SwiftPM
        // answers by refusing to let anyone depend on this package.
        //
        // MigoAppleRenderer names it. That matters beyond the renderer: while
        // no target depended on this binary target, SwiftPM never had to find a
        // slice for the platform being built, so `swift build` succeeded on an
        // xcframework containing only iOS and said nothing about it. The lane
        // that builds this package was therefore green without checking the
        // artifact it exists to check. scripts/test-apple-shipping-package-contract.sh
        // keeps at least one target depending on it, because losing that
        // property costs nothing at the time and silently un-checks the lane.
        .binaryTarget(
            name: "MigoEngine",
            path: "Frameworks/MigoEngine.xcframework"
        ),

        // Internal. CAMetalLayer ownership, surface attach/update/retire, and
        // the display link. Not a product: it is shared by two lanes and is not
        // a supported thing to depend on directly.
        .target(
            name: "MigoAppleRenderer",
            dependencies: [.product(name: "MigoAppleCore", package: "core"), "MigoEngine"],
            path: "Sources/MigoAppleRenderer"
        ),

        // Runs only where the xcframework has a slice for the host, which is
        // the macOS build. That is not a gap: the assertions are about the ABI
        // -- record sizes, a fail-closed write, a bitmask -- and the arm64
        // macOS slice and the arm64 iOS slice are the same Rust compiled for
        // the same architecture. What the iOS slice needs proved about it is
        // that it compiles and links for iOS, which is what the iOS legs of
        // .github/workflows/apple-sdk.yml do with xcodebuild.
        .testTarget(
            name: "MigoAppleRendererTests",
            dependencies: ["MigoAppleRenderer", "MigoEngine"],
            path: "Tests/MigoAppleRendererTests"
        ),

        // Lane 1: the compatibility and safety baseline. WKWebView runs the
        // JavaScript, WebKit renders. Deliberately does not depend on
        // MigoAppleRenderer -- linking a renderer it never drives would put
        // ANGLE and Skia into an app that asked for the opposite.
        .target(
            name: "MigoAppleWebKit",
            dependencies: [.product(name: "MigoAppleCore", package: "core")],
            path: "Sources/MigoAppleWebKit"
        ),

        // Lane 2: content JavaScript stays in WebKit's WebContent process;
        // G0 selects Window versus Dedicated Worker, transport, clock, and
        // host shape from the ProbeApp evidence.
        .target(
            name: "MigoApplePerformancePlus",
            dependencies: [.product(name: "MigoAppleCore", package: "core"), "MigoAppleRenderer", "MigoAppleWebKit"],
            path: "Sources/MigoApplePerformancePlus",
            // Generated, not authored here. The producer's source lives in
            // platforms/apple/WebContent/PerformancePlus so it can be bundled
            // and unit-tested with node, outside SwiftPM; build-apple-sdk.sh
            // emits the bundle into this directory. SwiftPM refuses resource
            // paths outside the target, and reaching outside is also what
            // would let the shipped bundle drift from the tested one.
            resources: [.copy("Resources")]
        ),

        // Lane 3: macOS only. In-process V8 with JIT; no second process.
        .target(
            name: "MigoMacV8",
            dependencies: [.product(name: "MigoAppleCore", package: "core"), "MigoAppleRenderer"],
            path: "Sources/MigoMacV8"
        ),
    ]
)
