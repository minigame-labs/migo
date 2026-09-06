import MigoEngine
import QuartzCore
import XCTest

#if os(macOS)
    import AppKit
#endif

/// A host-owned `CAMetalLayer` survives the whole C ABI attach path.
///
/// WHAT THIS PROVES, precisely, because the constant it feeds is guarded by a
/// rule about evidence and a vague claim here would be worse than none: a real
/// `CAMetalLayer` -- not a pointer-shaped integer -- is copied out of a versioned
/// descriptor, validated, matched to the Apple platform module, turned into an
/// `AppleMetalLayerSurface` and a `CAMetalLayer` graphics platform, and accepted
/// by a Session as generation 1. Every one of those steps is code this repository
/// owns and none of it had ever run on an Apple machine.
///
/// WHAT IT DOES NOT PROVE, and this is deliberate rather than an omission. It
/// does not prove ANGLE loaded, that EGL got a context, or that anything was
/// drawn. The shipped Apple product is `--features external-frames`, and
/// `engine/crates/core/src/runtime/external.rs` destructures `gpu_init_started`
/// into `_gpu_init_started` on purpose: an external-frame session reports ready
/// once its tokio runtime is up and never waits for the renderer, because its
/// producer needs somewhere to put the first packet before the GPU is warm. So
/// `MIGO_OK` here is a statement about the boundary, not about the GPU. The EGL
/// half is asserted separately and directly by
/// `angle_loads_under_its_pinned_name_and_answers_with_a_display` in
/// `engine/crates/platform/src/apple/presenter.rs`, on this same lane.
///
/// Saying that out loud matters because the failure this pair exists to prevent
/// is exactly a green signal that meant less than it looked like: a published
/// Windows SDK exported every entry point, loaded cleanly, and could attach
/// nothing, while every gate agreed with every other gate.
///
/// WHY SWIFT AND NOT A RUST TEST. The thing that has to be attached is a real
/// `CAMetalLayer`, which is what a host passes and what ANGLE's Metal backend
/// requires -- handed a plain `CALayer`, ANGLE allocates its own metal layer as a
/// sublayer and the host loses control of `maximumDrawableCount`, `contentsScale`
/// and `presentsWithTransaction`. Creating one from Rust would mean an
/// Objective-C crate this repository does not otherwise need; creating one here
/// needs nothing, because this test target already links the engine and already
/// runs on the macOS lane.
final class MigoSurfaceAttachTests: XCTestCase {
    #if os(macOS)
        private var root: URL!
        private var engine: OpaquePointer!
        private var session: OpaquePointer!
        private var attachment: OpaquePointer?

        /// Native objects the engine was handed a raw pointer to.
        ///
        /// Held by the fixture rather than by the test method, because two header
        /// contracts forbid releasing them where a local would be released.
        /// `migo_surface_begin_detach` is explicit that MIGO_OK means retirement
        /// STARTED: "Keep the native resource and its event loop alive until
        /// migo_surface_release_query reports MIGO_SURFACE_RELEASE_RELEASED;
        /// destroying it earlier is a use-after-free inside the driver, which the
        /// engine cannot detect or prevent." And `migo_engine_destroy` is the
        /// stricter, final gate -- it is a thread-completion barrier, and "only
        /// after it returns may the host destroy native display/window resources".
        ///
        /// A `CAMetalLayer` created as a local in the test method is released by
        /// ARC the moment that method returns -- before teardown has retired
        /// anything -- and `Unmanaged.passUnretained` does not retain, so nothing
        /// else keeps it alive. This array is cleared last, after
        /// `migo_engine_destroy` has returned.
        private var retained: [AnyObject] = []

        override func setUpWithError() throws {
            try super.setUpWithError()

            // Directories the engine may write into. A test that pointed these
            // at the source tree would be a test that leaves artefacts behind on
            // the machine that ran it.
            root = URL(fileURLWithPath: NSTemporaryDirectory())
                .appendingPathComponent("migo-attach-\(UUID().uuidString)")
            for name in ["files", "cache", "code-cache"] {
                try FileManager.default.createDirectory(
                    at: root.appendingPathComponent(name),
                    withIntermediateDirectories: true)
            }

            var created: OpaquePointer?
            try root.appendingPathComponent("files").path.withCString { files in
                try root.appendingPathComponent("cache").path.withCString { cache in
                    try root.appendingPathComponent("code-cache").path.withCString { codeCache in
                        var config = MigoEngineConfig()
                        config.struct_size = UInt32(MemoryLayout<MigoEngineConfig>.size)
                        config.abi_version = MIGO_ABI_VERSION_CURRENT
                        config.files_dir_utf8 = files
                        config.cache_dir_utf8 = cache
                        config.code_cache_dir_utf8 = codeCache
                        let result = migo_engine_create(&config, &created)
                        XCTAssertEqual(result, MIGO_OK, "migo_engine_create returned \(result)")
                    }
                }
            }
            engine = try XCTUnwrap(created)

            var sessionConfig = MigoSessionConfig()
            sessionConfig.struct_size = UInt32(MemoryLayout<MigoSessionConfig>.size)
            sessionConfig.abi_version = MIGO_ABI_VERSION_CURRENT
            // Required, and required by THIS product rather than by attach in
            // general. The shipped Apple xcframework is built
            // `--no-default-features --features external-frames`, and that lane
            // refuses to start a session with no launch identity: an all-zero
            // nonce is what an uninitialised struct holds, so a session that
            // accepted it would accept frame packets from any other process that
            // also failed to initialise one. Without this, attach returns
            // MIGO_ERROR_INVALID_STATE and never reaches the platform layer these
            // tests are about.
            //
            // Any non-zero value will do, because nothing here submits a packet.
            // A fixed one rather than a random one, so a failure reproduces.
            withUnsafeMutableBytes(of: &sessionConfig.launch_nonce) { bytes in
                bytes.copyBytes(from: CollectionOfOne(UInt8(0xA2)))
            }
            var startedSession: OpaquePointer?
            let result = migo_session_create(engine, &sessionConfig, &startedSession)
            XCTAssertEqual(result, MIGO_OK, "migo_session_create returned \(result)")
            session = try XCTUnwrap(startedSession)
        }

        /// The documented shutdown handshake, in full, and asserted at every
        /// step.
        ///
        /// Not housekeeping -- this is the half of the surface contract a host
        /// must implement, and running it here is the only place it is exercised
        /// on Apple. It also has to be done properly for the tests above to mean
        /// anything: `migo_session_destroy`'s header says destruction "refuses
        /// with MIGO_ERROR_INVALID_STATE while a Surface transition is active, an
        /// attachment is still live, or any retired Surface is still PENDING". A
        /// teardown that called destroy with a live attachment and ignored the
        /// result would leak the Session and its host thread once per test, and
        /// leave that thread holding a pointer to a deallocated layer.
        override func tearDownWithError() throws {
            if let live = attachment {
                attachment = nil
                var release: OpaquePointer?
                let began = migo_surface_begin_detach(live, &release)
                XCTAssertEqual(began, MIGO_OK, "migo_surface_begin_detach returned \(began)")

                if let release {
                    // Polled, not waited on: the header says the query never
                    // blocks precisely so a host can ask from its UI thread or an
                    // idle handler. A host's obligation is to keep asking while
                    // its event loop runs, so that is what this imitates.
                    var status = MigoSurfaceReleaseStatus()
                    var released = false
                    let deadline = Date().addingTimeInterval(5)
                    while Date() < deadline {
                        let queried = migo_surface_release_query(release, &status)
                        XCTAssertEqual(queried, MIGO_OK, "release_query returned \(queried)")
                        if status.state == MIGO_SURFACE_RELEASE_RELEASED {
                            released = true
                            break
                        }
                        usleep(1000)
                    }
                    XCTAssertTrue(
                        released,
                        """
                        the retired surface never reported RELEASED. Until it does, \
                        migo_session_destroy is required to refuse and the native layer \
                        is not safe to free, so a host in this state cannot shut down.
                        """)
                    let destroyed = migo_surface_release_destroy(release)
                    XCTAssertEqual(
                        destroyed, MIGO_OK, "migo_surface_release_destroy returned \(destroyed)")
                }
            }

            if let session {
                let result = migo_session_destroy(session)
                XCTAssertEqual(
                    result, MIGO_OK,
                    "migo_session_destroy returned \(result); a refusal means something was "
                        + "still attached or still PENDING")
                self.session = nil
            }
            if let engine {
                let result = migo_engine_destroy(engine)
                XCTAssertEqual(result, MIGO_OK, "migo_engine_destroy returned \(result)")
                self.engine = nil
            }
            // Last, and only now: `migo_engine_destroy` is the thread-completion
            // barrier, and its header says only after it returns may the host
            // destroy native display or window resources.
            retained.removeAll()
            if let root { try? FileManager.default.removeItem(at: root) }
            root = nil
            try super.tearDownWithError()
        }

        /// One attach, driven from a caller-owned payload of the given kind.
        ///
        /// `platform_descriptor_size` is required to equal the typed
        /// descriptor's own `struct_size`; the header calls that duplication an
        /// intentional envelope-versus-payload cross-check, so it is written from
        /// the same expression rather than typed twice.
        ///
        /// `hostObject` is the native object the payload points at, and taking it
        /// here is what guarantees it outlives the attachment: the fixture holds
        /// it until after teardown has seen RELEASED. A successful attachment is
        /// recorded on the fixture for the same reason -- it has to be retired
        /// before the Session may be destroyed.
        private func attach<Payload>(
            kind: MigoPlatformKind,
            payload: inout Payload,
            payloadSize: UInt32,
            hostObject: AnyObject
        ) -> MigoResult {
            retained.append(hostObject)
            var produced: OpaquePointer?
            let result = withUnsafePointer(to: &payload) { raw -> MigoResult in
                var descriptor = MigoSurfaceDescriptor()
                descriptor.struct_size = UInt32(MemoryLayout<MigoSurfaceDescriptor>.size)
                descriptor.abi_version = MIGO_ABI_VERSION_CURRENT
                // Generations start at 1 and must strictly increase per session.
                descriptor.generation = 1
                descriptor.platform_kind = kind
                descriptor.width_pixels = 256
                descriptor.height_pixels = 256
                descriptor.scale_factor = 1.0
                descriptor.color_space = MIGO_COLOR_SPACE_SRGB
                // OPAQUE, and not PREMULTIPLIED, which is what a layer-backed
                // renderer would reach for first. `validate_configuration`
                // answers PREMULTIPLIED and POSTMULTIPLIED with
                // MIGO_ERROR_UNSUPPORTED_CAPABILITY on purpose -- capability bits
                // and modes are requirements rather than hints, and the renderer
                // has not plumbed alpha semantics end to end -- so asking for it
                // here would fail during configuration validation and never reach
                // the platform layer this test is about. The same reasoning fixes
                // `capability_flags` at zero: any non-zero bit is refused for the
                // same documented reason.
                descriptor.alpha_mode = MIGO_ALPHA_MODE_OPAQUE
                descriptor.preferred_presentation_mode = MIGO_PRESENTATION_MODE_DEFAULT
                descriptor.platform_descriptor_size = payloadSize
                descriptor.platform_descriptor = UnsafeRawPointer(raw)
                return migo_session_attach_surface(session, &descriptor, &produced)
            }
            attachment = produced
            return result
        }
    #endif

    /// A host-owned `CAMetalLayer` is accepted, and the engine reports the
    /// attachment rather than merely not failing.
    func testAHostOwnedMetalLayerAttaches() throws {
        #if os(macOS)
            let layer = CAMetalLayer()
            layer.drawableSize = CGSize(width: 256, height: 256)
            layer.frame = CGRect(x: 0, y: 0, width: 256, height: 256)

            var payload = MigoMacosMetalLayerDescriptor()
            payload.struct_size = UInt32(MemoryLayout<MigoMacosMetalLayerDescriptor>.size)
            payload.abi_version = MIGO_ABI_VERSION_CURRENT
            payload.platform_kind = MIGO_PLATFORM_MACOS_CA_METAL_LAYER
            payload.ca_metal_layer = Unmanaged.passUnretained(layer).toOpaque()

            let result = attach(
                kind: MIGO_PLATFORM_MACOS_CA_METAL_LAYER,
                payload: &payload,
                payloadSize: payload.struct_size,
                hostObject: layer)

            XCTAssertEqual(
                result, MIGO_OK,
                """
                attaching a host-owned CAMetalLayer returned \(result). This is the \
                assertion that earns MIGO_PLATFORM_MACOS_CA_METAL_LAYER its place in \
                MIGO_CAPI_IMPLEMENTED_PLATFORM_KINDS; until it passes on this runner, \
                that constant must not list it.
                """)
            XCTAssertNotNil(attachment, "attach reported success and produced no attachment")
        #else
            throw XCTSkip("the attach path under test is the macOS one")
        #endif
    }

    /// The other half of the same decision: a view is refused, not resolved.
    ///
    /// `MigoMacosNsViewDescriptor` is a payload the ABI parses and the platform
    /// module deliberately declines, because ANGLE handed a plain `CALayer`
    /// succeeds and quietly takes ownership of a metal layer it created itself. A
    /// refusal is the only outcome that leaves the host in charge of its own
    /// drawable, so it is asserted rather than assumed -- and asserting it here is
    /// what separates "attach works" from "attach accepts whatever it is given".
    ///
    /// WHICH refusal this reaches, precisely: the capability mask's, not the
    /// platform module's match arm. `parse_for_platforms` is handed
    /// `supported_platform_kinds()`, which carries only this build's layer kind,
    /// so a view descriptor is turned away before any Apple code sees it. The
    /// module's own arm is covered by `a_view_descriptor_is_refused_rather_than_
    /// resolved_to_a_layer` in `capi/src/platform/apple.rs`, which runs on Linux.
    /// Two independent refusals for one rule, which is the intent -- but worth
    /// naming, so nobody reads this test as proof of the arm it does not reach.
    func testAnNsViewIsRefusedRatherThanResolvedToALayer() throws {
        #if os(macOS)
            let view = NSView(frame: CGRect(x: 0, y: 0, width: 256, height: 256))

            var payload = MigoMacosNsViewDescriptor()
            payload.struct_size = UInt32(MemoryLayout<MigoMacosNsViewDescriptor>.size)
            payload.abi_version = MIGO_ABI_VERSION_CURRENT
            payload.platform_kind = MIGO_PLATFORM_MACOS_NS_VIEW
            payload.ns_view = Unmanaged.passUnretained(view).toOpaque()

            let result = attach(
                kind: MIGO_PLATFORM_MACOS_NS_VIEW,
                payload: &payload,
                payloadSize: payload.struct_size,
                hostObject: view)

            XCTAssertEqual(
                result, MIGO_ERROR_UNSUPPORTED_PLATFORM,
                "an NSView must be declined so the host keeps ownership of its drawable")
            XCTAssertNil(attachment, "a declined attach must produce no attachment")
        #else
            throw XCTSkip("the attach path under test is the macOS one")
        #endif
    }
}
