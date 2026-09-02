import XCTest

@testable import MigoAppleCore

final class MigoDeploymentFloorTests: XCTestCase {
    /// A lane minimum below the deployment floor would be unreachable code that
    /// still reads as a supported configuration. The relationship, not the
    /// numbers, is the invariant: raising the deployment floor past a lane
    /// minimum must be noticed, not silently absorbed.
    func testEveryLaneMinimumIsAtOrAboveTheDeploymentFloor() {
        let floor = MigoDeploymentFloor.iOS
        let lane = MigoDeploymentFloor.performancePlusMinimumIOS
        XCTAssertTrue(
            lane.major > floor.major || (lane.major == floor.major && lane.minor >= floor.minor),
            "Performance+ minimum \(lane) is below the iOS deployment floor \(floor)"
        )
    }

    /// Not an assertion that the current numbers are right -- the contract file
    /// decides that, and scripts/test-apple-deployment-floor-contract.sh checks
    /// this file against it. This asserts they are *parsed* as versions at all,
    /// so a zeroed constant cannot pass the comparison above by being smaller
    /// than everything.
    func testFloorsAreRealVersions() {
        XCTAssertGreaterThan(MigoDeploymentFloor.iOS.major, 0)
        XCTAssertGreaterThan(MigoDeploymentFloor.macOS.major, 0)
    }
}
