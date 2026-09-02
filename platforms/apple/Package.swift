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
        // Three shipping products, and no fourth.
        //
        // Each one carries exactly one JavaScript execution model, because the
        // reason to split them is the dependency closure: a host that wants the
        // compatibility baseline should not link ANGLE, Skia and a renderer it
        // will never call, and a host that wants Performance+ must be able to
        // prove by inspection that no JavaScript engine is linked into its app
        // process at all. An umbrella product would make both claims unprovable.
        .library(name: "MigoAppleWebKit", targets: ["MigoAppleWebKit"]),
        .library(name: "MigoApplePerformancePlus", targets: ["MigoApplePerformancePlus"]),
        .library(name: "MigoMacV8", targets: ["MigoMacV8"]),
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
        .binaryTarget(
            name: "MigoEngine",
            path: "Frameworks/MigoEngine.xcframework"
        ),

        // Shared, engine-agnostic host services: profile resolution, lifecycle,
        // permissions, metrics. Nothing here knows which lane will be selected.
        .target(
            name: "MigoAppleCore",
            dependencies: ["MigoEngine"],
            path: "Sources/MigoAppleCore"
        ),

        // Internal. CAMetalLayer ownership, surface attach/update/retire, and
        // the display link. Not a product: it is shared by two lanes and is not
        // a supported thing to depend on directly.
        .target(
            name: "MigoAppleRenderer",
            dependencies: ["MigoAppleCore"],
            path: "Sources/MigoAppleRenderer"
        ),

        // Lane 1: the compatibility and safety baseline. WKWebView runs the
        // JavaScript, WebKit renders. Deliberately does not depend on
        // MigoAppleRenderer -- linking a renderer it never drives would put
        // ANGLE and Skia into an app that asked for the opposite.
        .target(
            name: "MigoAppleWebKit",
            dependencies: ["MigoAppleCore"],
            path: "Sources/MigoAppleWebKit"
        ),

        // Lane 2: content JavaScript runs in a Worker inside WebKit's
        // WebContent process, and rendering comes back here.
        .target(
            name: "MigoApplePerformancePlus",
            dependencies: ["MigoAppleCore", "MigoAppleRenderer", "MigoAppleWebKit"],
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
            dependencies: ["MigoAppleCore", "MigoAppleRenderer"],
            path: "Sources/MigoMacV8"
        ),

        .testTarget(
            name: "MigoAppleCoreTests",
            dependencies: ["MigoAppleCore"],
            path: "Tests/MigoAppleCoreTests"
        ),
    ]
)
