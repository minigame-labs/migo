//! Cold-path platform boundary for EGL loading and native surface creation.
//!
//! The provider/factory pair is validated once with process-local type identity.
//! No method in this module participates in draw or present hot paths.

use std::{
    any::{Any, TypeId},
    fmt::Debug,
    sync::Arc,
};

use khronos_egl as egl;
use shared::{
    error::{EngineError, EngineResult, ErrorCode},
    surface::Surface,
};

pub type EglInstance = egl::DynamicInstance<egl::EGL1_4>;

/// Process-local backend identity derived only from a private concrete marker.
/// It is deliberately not a serialized/public ABI identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GraphicsBackendId(TypeId);

impl GraphicsBackendId {
    pub fn of<T: 'static>() -> Self {
        Self(TypeId::of::<T>())
    }
}

pub trait EglProvider: Debug + Send + Sync {
    fn backend_id(&self) -> GraphicsBackendId;
    fn label(&self) -> &str;
    fn load(&self) -> EngineResult<EglInstance>;
    fn display(&self, egl: &EglInstance) -> EngineResult<egl::Display>;
}

pub trait EglSurfaceFactory: Debug + Send + Sync {
    fn backend_id(&self) -> GraphicsBackendId;

    /// Convert the platform Surface into a non-owning presenter target.
    /// A failed concrete downcast must return `Unsupported`.
    fn prepare(&self, surface: &dyn Surface) -> EngineResult<PreparedEglSurfaceRef>;
}

pub trait PreparedEglSurface: Debug + Send + Sync {
    fn backend_id(&self) -> GraphicsBackendId;
    fn as_any(&self) -> &dyn Any;

    /// Return false when concrete types differ or a downcast fails.
    fn same_native_surface(&self, other: &dyn PreparedEglSurface) -> bool;

    /// Repeatable native surface creation. Returning `Err` guarantees this
    /// call retained no newly-created EGL object.
    fn create_window_surface(
        &self,
        egl: &EglInstance,
        display: egl::Display,
        config: egl::Config,
    ) -> EngineResult<egl::Surface>;
}

pub type PreparedEglSurfaceRef = Arc<dyn PreparedEglSurface>;

#[derive(Clone, Debug)]
pub struct GraphicsPlatform {
    egl_provider: Arc<dyn EglProvider>,
    surface_factory: Arc<dyn EglSurfaceFactory>,
    backend_id: GraphicsBackendId,
}

impl GraphicsPlatform {
    pub fn try_new(
        egl_provider: Arc<dyn EglProvider>,
        surface_factory: Arc<dyn EglSurfaceFactory>,
    ) -> EngineResult<Self> {
        let provider_id = egl_provider.backend_id();
        let factory_id = surface_factory.backend_id();
        if provider_id != factory_id {
            return Err(EngineError::new(ErrorCode::InvalidOperation)
                .with_msg("incompatible EGL provider and surface factory")
                .with_detail(format!(
                    "provider={} ({provider_id:?}), factory={factory_id:?}",
                    egl_provider.label()
                )));
        }
        Ok(Self {
            egl_provider,
            surface_factory,
            backend_id: provider_id,
        })
    }

    #[inline]
    pub fn backend_id(&self) -> GraphicsBackendId {
        self.backend_id
    }

    #[inline]
    pub fn egl_provider(&self) -> &Arc<dyn EglProvider> {
        &self.egl_provider
    }

    #[inline]
    pub fn surface_factory(&self) -> &Arc<dyn EglSurfaceFactory> {
        &self.surface_factory
    }

    pub fn prepare_surface(&self, surface: &dyn Surface) -> EngineResult<PreparedEglSurfaceRef> {
        let prepared = self.surface_factory.prepare(surface)?;
        self.validate_prepared(prepared.as_ref())?;
        Ok(prepared)
    }

    pub fn validate_prepared(&self, prepared: &dyn PreparedEglSurface) -> EngineResult<()> {
        if prepared.backend_id() != self.backend_id {
            return Err(EngineError::new(ErrorCode::Unsupported)
                .with_msg("prepared EGL surface belongs to another backend")
                .with_detail(format!(
                    "platform={:?}, prepared={:?}",
                    self.backend_id,
                    prepared.backend_id()
                )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        marker::PhantomData,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use super::*;

    #[derive(Debug)]
    struct BackendA;
    #[derive(Debug)]
    struct BackendB;

    #[derive(Debug)]
    struct FakeProvider<T> {
        marker: PhantomData<fn() -> T>,
    }

    impl<T> FakeProvider<T> {
        fn new() -> Self {
            Self {
                marker: PhantomData,
            }
        }
    }

    impl<T: 'static + Debug> EglProvider for FakeProvider<T> {
        fn backend_id(&self) -> GraphicsBackendId {
            GraphicsBackendId::of::<T>()
        }

        fn label(&self) -> &str {
            std::any::type_name::<T>()
        }

        fn load(&self) -> EngineResult<EglInstance> {
            Err(EngineError::new(ErrorCode::RenderInitializeError))
        }

        fn display(&self, _egl: &EglInstance) -> EngineResult<egl::Display> {
            Err(EngineError::new(ErrorCode::RenderInitializeError))
        }
    }

    #[derive(Debug)]
    struct FakeFactory<FactoryBackend, TargetBackend> {
        native_creates: Arc<AtomicUsize>,
        marker: PhantomData<fn() -> (FactoryBackend, TargetBackend)>,
    }

    impl<F, T> FakeFactory<F, T> {
        fn new(native_creates: Arc<AtomicUsize>) -> Self {
            Self {
                native_creates,
                marker: PhantomData,
            }
        }
    }

    impl<F: 'static + Debug, T: 'static + Debug> EglSurfaceFactory for FakeFactory<F, T> {
        fn backend_id(&self) -> GraphicsBackendId {
            GraphicsBackendId::of::<F>()
        }

        fn prepare(&self, _surface: &dyn Surface) -> EngineResult<PreparedEglSurfaceRef> {
            Ok(Arc::new(FakePrepared::<T> {
                native_creates: Arc::clone(&self.native_creates),
                marker: PhantomData,
            }))
        }
    }

    #[derive(Debug)]
    struct FakePrepared<T> {
        native_creates: Arc<AtomicUsize>,
        marker: PhantomData<fn() -> T>,
    }

    impl<T: 'static + Debug> PreparedEglSurface for FakePrepared<T> {
        fn backend_id(&self) -> GraphicsBackendId {
            GraphicsBackendId::of::<T>()
        }

        fn as_any(&self) -> &dyn Any {
            self
        }

        fn same_native_surface(&self, other: &dyn PreparedEglSurface) -> bool {
            other.as_any().downcast_ref::<Self>().is_some()
        }

        fn create_window_surface(
            &self,
            _egl: &EglInstance,
            _display: egl::Display,
            _config: egl::Config,
        ) -> EngineResult<egl::Surface> {
            self.native_creates.fetch_add(1, Ordering::Relaxed);
            Err(EngineError::new(ErrorCode::RenderBackendError))
        }
    }

    #[derive(Debug)]
    struct TestSurface;

    impl Surface for TestSurface {
        fn as_any(&self) -> &dyn Any {
            self
        }

        fn size(&self) -> (u32, u32) {
            (1, 1)
        }
    }

    #[test]
    fn rejects_mismatched_provider_and_factory_identity() {
        let err = GraphicsPlatform::try_new(
            Arc::new(FakeProvider::<BackendA>::new()),
            Arc::new(FakeFactory::<BackendB, BackendB>::new(Arc::new(
                AtomicUsize::new(0),
            ))),
        )
        .unwrap_err();

        assert_eq!(err.code, ErrorCode::InvalidOperation);
    }

    #[test]
    fn accepts_matched_pair_and_exposes_provider_label() {
        let platform = GraphicsPlatform::try_new(
            Arc::new(FakeProvider::<BackendA>::new()),
            Arc::new(FakeFactory::<BackendA, BackendA>::new(Arc::new(
                AtomicUsize::new(0),
            ))),
        )
        .unwrap();

        assert_eq!(platform.backend_id(), GraphicsBackendId::of::<BackendA>());
        assert!(platform.egl_provider().label().ends_with("BackendA"));
    }

    #[test]
    fn rejects_factory_output_from_another_backend_before_native_creation() {
        let native_creates = Arc::new(AtomicUsize::new(0));
        let platform = GraphicsPlatform::try_new(
            Arc::new(FakeProvider::<BackendA>::new()),
            Arc::new(FakeFactory::<BackendA, BackendB>::new(Arc::clone(
                &native_creates,
            ))),
        )
        .unwrap();

        let err = platform.prepare_surface(&TestSurface).unwrap_err();
        assert_eq!(err.code, ErrorCode::Unsupported);
        assert_eq!(native_creates.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn cross_type_native_comparison_fails_closed() {
        let creates = Arc::new(AtomicUsize::new(0));
        let a = FakePrepared::<BackendA> {
            native_creates: Arc::clone(&creates),
            marker: PhantomData,
        };
        let b = FakePrepared::<BackendB> {
            native_creates: creates,
            marker: PhantomData,
        };

        assert!(!a.same_native_surface(&b));
    }
}
