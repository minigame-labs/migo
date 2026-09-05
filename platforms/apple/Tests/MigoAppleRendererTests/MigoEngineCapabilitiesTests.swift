import MigoEngine
import XCTest

@testable import MigoAppleRenderer

/// The first tests in this repository that execute the Migo C ABI on Apple
/// hardware.
///
/// Everything Apple-facing here has been checked by comparing text against a
/// contract, or by asking a compiler to agree. Neither can answer whether the
/// library that was actually built and lipo'd into the xcframework answers a
/// call -- and a library that links, exports everything and can attach nothing
/// is not hypothetical in this repository; it shipped once, as
/// windows-sdk-0.1.0.
///
/// THE RULE FOLLOWED THROUGHOUT: nothing here pins today's absence. `migo-capi`
/// selects its `unsupported` platform module for every Apple target, so the
/// live library advertises no attachable kind, and an assertion that said so
/// would go red on the day the presenter lands. Every verdict about a specific
/// mask is driven by a synthetic `MigoEngineCapabilities` the test constructs;
/// what is asserted about the live library is only what stays true after the
/// port exists.
final class MigoEngineCapabilitiesTests: XCTestCase {
    // MARK: - The live library

    /// The call reaches the linked library and the library answers.
    ///
    /// The version assertion is a cross-check between two things that were
    /// built separately and could disagree: `MIGO_ABI_VERSION_CURRENT` comes
    /// from the headers the xcframework carries, and the range comes from the
    /// compiled Rust. A library that does not accept the version its own
    /// shipped headers call current is one no correct host could call.
    func testTheLinkedLibraryAnswersTheCapabilityQuery() throws {
        let capabilities = try MigoEngineCapabilities.query()

        XCTAssertLessThanOrEqual(
            capabilities.acceptedABIVersionMin,
            capabilities.acceptedABIVersionMax,
            "the library reported an empty ABI version range, which no caller can satisfy"
        )
        XCTAssertTrue(
            capabilities.accepts(abiVersion: MIGO_ABI_VERSION_CURRENT),
            """
            the linked library does not accept MIGO_ABI_VERSION_CURRENT \
            (\(MIGO_ABI_VERSION_CURRENT)), which is the version declared by the \
            headers shipped alongside it in the same xcframework
            """
        )
    }

    /// The fail-closed half of the record contract, executed rather than
    /// asserted about.
    ///
    /// `struct_size` governs how many bytes the library may write into the
    /// caller's storage, so an undersized record must be refused and must
    /// leave every byte alone -- a partial write is the failure that reads as
    /// a successful query reporting nothing. The header states both halves;
    /// this is the first time either has been observed on Apple.
    func testAnUndersizedRecordIsRefusedAndNothingIsWritten() {
        var record = MigoCapabilities()
        // Deliberately not a plausible size: smaller than the record's own
        // header, so no policy could read it as an older, shorter version.
        record.struct_size = 4
        record.abi_version = MIGO_ABI_VERSION_CURRENT
        record.abi_version_min = 0xA1A1_A1A1
        record.abi_version_max = 0xB2B2_B2B2
        record.platform_kinds = 0xC3C3_C3C3_C3C3_C3C3

        let result = migo_query_capabilities(&record)

        XCTAssertEqual(result, MIGO_ERROR_INVALID_ARGUMENT)
        XCTAssertEqual(record.abi_version_min, 0xA1A1_A1A1, "a rejected query wrote to the caller's record")
        XCTAssertEqual(record.abi_version_max, 0xB2B2_B2B2, "a rejected query wrote to the caller's record")
        XCTAssertEqual(record.platform_kinds, 0xC3C3_C3C3_C3C3_C3C3, "a rejected query wrote to the caller's record")
    }

    /// The renderer's verdict about the library it is linked to must be the one
    /// the library's own answer implies.
    ///
    /// Written as an implication rather than an expected value so it holds
    /// before and after the Apple presenter exists: today it exercises the
    /// refusal arm, and on the day `supported_platform_kinds()` stops returning
    /// zero for Apple targets it starts exercising the ready arm, with no edit.
    func testThePreflightVerdictFollowsTheLiveCapabilityMask() throws {
        let capabilities = try MigoEngineCapabilities.query()
        let kind = MigoEngineCapabilities.hostSurfaceKind
        let verdict = capabilities.preflight()

        if capabilities.accepts(abiVersion: MIGO_ABI_VERSION_CURRENT),
            capabilities.canAttach(surfaceKind: kind)
        {
            XCTAssertEqual(verdict, .ready)
        } else {
            XCTAssertNotEqual(
                verdict, .ready,
                "preflight reported ready for a kind (\(kind)) the library does not advertise"
            )
        }
    }

    // MARK: - The verdict, on masks the test controls

    func testPreflightIsReadyWhenTheLibraryAdvertisesTheKind() {
        let capabilities = MigoEngineCapabilities(
            acceptedABIVersionMin: MIGO_ABI_VERSION_CURRENT,
            acceptedABIVersionMax: MIGO_ABI_VERSION_CURRENT,
            attachableSurfaceKinds: 1 << UInt64(MigoEngineCapabilities.hostSurfaceKind)
        )

        XCTAssertEqual(capabilities.preflight(), .ready)
    }

    func testPreflightRefusesAKindTheLibraryDoesNotAdvertise() {
        // Every other kind advertised, so the refusal cannot come from an empty
        // mask -- it has to come from the one bit that matters.
        let kind = MigoEngineCapabilities.hostSurfaceKind
        let capabilities = MigoEngineCapabilities(
            acceptedABIVersionMin: MIGO_ABI_VERSION_CURRENT,
            acceptedABIVersionMax: MIGO_ABI_VERSION_CURRENT,
            attachableSurfaceKinds: ~(1 << UInt64(kind))
        )

        XCTAssertEqual(capabilities.preflight(), .surfaceKindNotAttachable(kind: kind))
    }

    /// When both preconditions fail, the ABI version is the one reported.
    ///
    /// Not a preference for one message over another: the platform-kind bit
    /// numbering is defined by the ABI version, so a mask read out of a library
    /// that rejects our version has no agreed meaning, and reporting a verdict
    /// derived from it would be reporting a guess as a fact.
    func testAnUnacceptedABIVersionIsReportedBeforeTheSurfaceKind() {
        let capabilities = MigoEngineCapabilities(
            acceptedABIVersionMin: MIGO_ABI_VERSION_CURRENT &+ 1,
            acceptedABIVersionMax: MIGO_ABI_VERSION_CURRENT &+ 2,
            attachableSurfaceKinds: 0
        )

        XCTAssertEqual(
            capabilities.preflight(),
            .abiVersionNotAccepted(
                hostRequires: MIGO_ABI_VERSION_CURRENT,
                acceptsMin: MIGO_ABI_VERSION_CURRENT &+ 1,
                acceptsMax: MIGO_ABI_VERSION_CURRENT &+ 2
            )
        )
    }

    /// The explicit overload decides against requirements the caller states,
    /// and the no-argument one against this renderer's own. One mask, two
    /// questions, two answers -- which is the whole reason a host that wants to
    /// know about a kind it is not presenting into can ask.
    func testPreflightAnswersForARequirementTheCallerStates() {
        // A kind this renderer never passes: the view descriptor, the
        // convenience path where the host gives up layer ownership. Named by
        // its constant rather than derived from `hostSurfaceKind` -- the two
        // happen to be adjacent numbers, and arithmetic on that would be a test
        // that passes for a reason the header never promised.
        #if os(iOS)
            let otherKind = MIGO_PLATFORM_IOS_UI_VIEW
        #else
            let otherKind = MIGO_PLATFORM_MACOS_NS_VIEW
        #endif
        let capabilities = MigoEngineCapabilities(
            acceptedABIVersionMin: MIGO_ABI_VERSION_CURRENT,
            acceptedABIVersionMax: MIGO_ABI_VERSION_CURRENT,
            attachableSurfaceKinds: 1 << UInt64(otherKind)
        )

        XCTAssertEqual(
            capabilities.preflight(hostRequires: MIGO_ABI_VERSION_CURRENT, surfaceKind: otherKind),
            .ready
        )
        XCTAssertEqual(
            capabilities.preflight(),
            .surfaceKindNotAttachable(kind: MigoEngineCapabilities.hostSurfaceKind)
        )
    }

    /// Every shape of constant the public headers use, named from Swift.
    ///
    /// Not a tautology, and not really about these particular flags. Swift's
    /// Clang importer reads a macro's definition tokens rather than its
    /// expansion, and it refuses shapes it does not model -- which is how all
    /// 103 constants in these headers came to be unnameable from Swift while
    /// being perfectly correct C. The headers now use four shapes, and a
    /// constant in a shape the importer declines is a constant no Apple host
    /// can write down.
    ///
    /// So one reference per shape, and this is what makes them assertions
    /// rather than mentions: each says something that has to stay true anyway.
    ///
    /// It lives in the test target rather than in the library on purpose.
    /// `xcodebuild` builds the product scheme without tests, so if the answer
    /// for one shape is no, all three platforms still compile and only this
    /// fails -- one fact lost instead of every fact in the run.
    func testEveryConstantShapeInTheHeadersIsNameableFromSwift() {
        // `<n>U`
        XCTAssertNotEqual(MIGO_PLATFORM_IOS_CA_METAL_LAYER, MIGO_PLATFORM_UNKNOWN)
        // `(1U << k)`
        XCTAssertNotEqual(MIGO_ERROR_FLAG_RECOVERABLE, MIGO_ERROR_FLAG_NONE)
        // `<n>ULL`
        XCTAssertEqual(MIGO_SURFACE_CAPABILITY_NONE, 0)
        // `(1ULL << k)`
        XCTAssertNotEqual(MIGO_SURFACE_CAPABILITY_WIDE_COLOR, MIGO_SURFACE_CAPABILITY_NONE)
    }

    // MARK: - The mask

    /// A kind outside the mask's width is unsupported by definition, and the
    /// control is the highest kind that is inside it.
    ///
    /// Both halves matter. Without the `63` case an implementation that always
    /// answered false would pass; without the `64` and `255` cases, one that
    /// let the shift wrap would. `migo-capi`'s `kind_is_supported` carries the
    /// same guard for the same reason, and the two agreeing is the point.
    func testAKindOutsideTheMaskWidthIsNeverAttachable() {
        let everything = MigoEngineCapabilities(
            acceptedABIVersionMin: MIGO_ABI_VERSION_CURRENT,
            acceptedABIVersionMax: MIGO_ABI_VERSION_CURRENT,
            attachableSurfaceKinds: UInt64.max
        )

        XCTAssertTrue(everything.canAttach(surfaceKind: 63))
        XCTAssertFalse(everything.canAttach(surfaceKind: 64))
        XCTAssertFalse(everything.canAttach(surfaceKind: 255))
        XCTAssertFalse(everything.canAttach(surfaceKind: UInt32.max))
    }

    /// This target owns the `CAMetalLayer` it presents into, so the kind it
    /// preflights is the layer kind. Stated as "not the view kind" rather than
    /// as the expected constant: repeating the constant would restate the
    /// implementation, while this catches the substitution that would actually
    /// happen -- the neighbouring descriptor, which compiles and attaches and
    /// hands `nextDrawable` a `UIView`.
    func testTheHostSurfaceKindIsALayerAndNotAView() {
        let kind = MigoEngineCapabilities.hostSurfaceKind

        XCTAssertNotEqual(kind, MIGO_PLATFORM_UNKNOWN)
        XCTAssertNotEqual(kind, MIGO_PLATFORM_IOS_UI_VIEW)
        XCTAssertNotEqual(kind, MIGO_PLATFORM_MACOS_NS_VIEW)
    }
}
