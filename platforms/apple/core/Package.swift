// swift-tools-version: 5.9
//
// The engine-free half of the Apple platform layer.
//
// WHY THIS IS A SEPARATE PACKAGE. The package next door declares a
// `.binaryTarget` for MigoEngine.xcframework, which is a build output of
// scripts/build-apple-sdk.sh -- a script that needs macOS, Xcode and an hour of
// Skia. Until it has run there is no artifact, so that package does not resolve,
// so nothing in it compiles. That is deliberate there and it is stated in its
// manifest. The consequence was not: it meant NO Swift in this repository had
// ever been seen by a compiler, including the deployment floors and the profile
// enum, both of which have contract gates that compare their *text* against
// contracts/apple/*.json and can therefore agree perfectly with the contract
// while failing to build.
//
// That is precisely the state apple-ci.yml exists to prevent, recorded there as
// "writing a platform arm twice is not testing it twice" -- the ILP32
// assertions in the C ABI lane had never been compiled, and two of them were
// wrong about pointer width once they were.
//
// So the boundary is drawn where the facts are, exactly as it is on the Rust
// side: `migo-frame-wire` / `migo-frame-decode` / `migo-capi-abi` are the crates
// that need no engine and therefore get checked on every PR, and these are the
// Swift sources that need no engine and now get the same treatment. `swift
// build` and `swift test` run here on the free macOS runner in seconds, with no
// xcframework and no Skia.
//
// WHAT MAY LIVE HERE is bounded by that promise and checked by
// scripts/test-apple-swift-core-engine-free.sh: no binary targets, no package
// dependencies, and no source that imports the engine module. Code that has to
// call the C ABI belongs in the shipping package, where the artifact it needs
// is declared.
//
// GENERATED PLATFORM FLOOR. The two `.iOS`/`.macOS` values below are derived
// from contracts/apple/deployment-floor.json and are checked against it by
// scripts/test-apple-deployment-floor-contract.sh. Change the contract, not
// this line: a hand-edited copy here is exactly the drift the gate exists to
// catch, and it drifts silently because a wrong-but-valid version still builds.

import PackageDescription

let package = Package(
    name: "MigoAppleCore",
    platforms: [
        .iOS(.v15),
        .macOS(.v11),
    ],
    products: [
        .library(name: "MigoAppleCore", targets: ["MigoAppleCore"]),
    ],
    targets: [
        // Shared, engine-agnostic host services: profile resolution, lifecycle,
        // permissions, metrics. Nothing here knows which lane will be selected,
        // and nothing here links the engine.
        .target(
            name: "MigoAppleCore",
            path: "Sources/MigoAppleCore"
        ),
        .testTarget(
            name: "MigoAppleCoreTests",
            dependencies: ["MigoAppleCore"],
            path: "Tests/MigoAppleCoreTests"
        ),
    ]
)
