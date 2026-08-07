//! Which EGL context a `glDelete*` must be issued in.
//!
//! ES 3.0 Appendix C.1 lists what an EGL share group shares: buffer, program,
//! shader, renderbuffer, sampler, sync and texture objects. It does **not** share
//! the container objects -- framebuffers, vertex arrays, queries and transform
//! feedbacks. Each context of the group has its own namespace for those, so the
//! same small integer names a different object in each, and drivers hand them out
//! from 1 upwards per context.
//!
//! The consequence is that deleting a container object from another context of the
//! group is one of two silent faults, and which one depends on the driver rather
//! than on the content: the name is unused there and `glDelete*` ignores it, so the
//! object leaks with its bookkeeping already discarded; or the name *is* live there
//! and another canvas's object is destroyed instead. On a Mesa-style driver, which
//! numbers container objects from one counter for the whole share group, the first
//! always happens and the second never can; on a driver that numbers per context,
//! an offscreen canvas's first framebuffer collides with the onscreen canvas's
//! DrawingBuffer.
//!
//! That decision used to be taken independently at each of the eleven delete sites,
//! in four different ways: some bound any canvas, some bound nothing at all, and
//! only queries and transform feedbacks consulted the owner -- with a fallback that
//! deleted from whatever context happened to be current when they could not. So
//! framebuffers and vertex arrays, the other two container kinds, were deleted from
//! the wrong context every time an offscreen canvas freed one while the onscreen
//! canvas was current.
//!
//! [`GlObject`] exists so the decision is taken once. A container variant cannot be
//! constructed without naming its owner, and a caller cannot issue the `glDelete*`
//! itself, because the handle goes in and only [`GlObject::delete`] takes it out.

use glow::{
    HasContext, NativeBuffer, NativeFence, NativeFramebuffer, NativeProgram, NativeQuery,
    NativeRenderbuffer, NativeSampler, NativeShader, NativeTexture, NativeTransformFeedback,
    NativeVertexArray,
};
use shared::protocol::render_cmd::CanvasId;

/// One GL object to be deleted, carrying its kind and its name together so the two
/// cannot disagree -- and, for the kinds whose name is context-local, the canvas
/// whose context minted it.
///
/// Constructing a container variant requires the owner, which is what makes
/// "deleted from the owning context" a property of the type rather than a rule each
/// delete site has to remember. See the module documentation for why that matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GlObject {
    // ---- Shared across the whole EGL share group (ES 3.0 Appendix C.1). Any
    // context of the group may delete these, so no owner is carried: naming one
    // would invite a caller to believe it mattered.
    Buffer(NativeBuffer),
    Program(NativeProgram),
    Renderbuffer(NativeRenderbuffer),
    Sampler(NativeSampler),
    Shader(NativeShader),
    Sync(NativeFence),
    Texture(NativeTexture),

    // ---- Container objects, not shared. The owner is part of the value.
    Framebuffer {
        handle: NativeFramebuffer,
        owner: CanvasId,
    },
    Query {
        handle: NativeQuery,
        owner: CanvasId,
    },
    TransformFeedback {
        handle: NativeTransformFeedback,
        owner: CanvasId,
    },
    VertexArray {
        handle: NativeVertexArray,
        owner: CanvasId,
    },
}

impl GlObject {
    /// The canvas whose context must be current for this deletion, or `None` when
    /// any context of the share group will do.
    ///
    /// This match has no catch-all, so a GL object kind cannot be added without its
    /// sharing being decided here.
    pub(crate) fn owning_context(&self) -> Option<CanvasId> {
        match self {
            GlObject::Buffer(_)
            | GlObject::Program(_)
            | GlObject::Renderbuffer(_)
            | GlObject::Sampler(_)
            | GlObject::Shader(_)
            | GlObject::Sync(_)
            | GlObject::Texture(_) => None,
            GlObject::Framebuffer { owner, .. }
            | GlObject::Query { owner, .. }
            | GlObject::TransformFeedback { owner, .. }
            | GlObject::VertexArray { owner, .. } => Some(*owner),
        }
    }

    /// Issue the `glDelete*`. The caller is responsible for having made
    /// [`Self::owning_context`] current; `CanvasManager::delete_gl_object` is the one
    /// place that pairs the two.
    pub(crate) fn delete(self, gl: &glow::Context) {
        unsafe {
            match self {
                GlObject::Buffer(h) => gl.delete_buffer(h),
                GlObject::Program(h) => gl.delete_program(h),
                GlObject::Renderbuffer(h) => gl.delete_renderbuffer(h),
                GlObject::Sampler(h) => gl.delete_sampler(h),
                GlObject::Shader(h) => gl.delete_shader(h),
                GlObject::Sync(h) => gl.delete_sync(h),
                GlObject::Texture(h) => gl.delete_texture(h),
                GlObject::Framebuffer { handle, .. } => gl.delete_framebuffer(handle),
                GlObject::Query { handle, .. } => gl.delete_query(handle),
                GlObject::TransformFeedback { handle, .. } => gl.delete_transform_feedback(handle),
                GlObject::VertexArray { handle, .. } => gl.delete_vertex_array(handle),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU32;

    fn owner() -> CanvasId {
        CanvasId::from(7u32)
    }

    fn nz(n: u32) -> NonZeroU32 {
        NonZeroU32::new(n).expect("test handle must be non-zero")
    }

    /// The four container kinds, each of which was previously decided somewhere
    /// else. Framebuffers and vertex arrays are the two this test was written for:
    /// their deletes consulted nothing at all, so an offscreen canvas freeing one
    /// while the onscreen canvas was current issued the call against the onscreen
    /// context's namespace.
    #[test]
    fn a_container_object_must_be_deleted_from_the_context_that_minted_it() {
        assert_eq!(
            GlObject::Framebuffer {
                handle: NativeFramebuffer(nz(1)),
                owner: owner(),
            }
            .owning_context(),
            Some(owner()),
        );
        assert_eq!(
            GlObject::VertexArray {
                handle: NativeVertexArray(nz(1)),
                owner: owner(),
            }
            .owning_context(),
            Some(owner()),
        );
        assert_eq!(
            GlObject::Query {
                handle: NativeQuery(nz(1)),
                owner: owner(),
            }
            .owning_context(),
            Some(owner()),
        );
        assert_eq!(
            GlObject::TransformFeedback {
                handle: NativeTransformFeedback(nz(1)),
                owner: owner(),
            }
            .owning_context(),
            Some(owner()),
        );
    }

    /// The positive control for the test above. Every assertion there is satisfied
    /// by a classification that answers `Some(owner)` for everything, which would
    /// cost a context switch on each of the seven shared kinds and -- worse -- would
    /// refuse to delete a shared object whose nominal owner canvas has gone, even
    /// though the object outlives that canvas by definition.
    #[test]
    fn a_shared_object_may_be_deleted_from_any_context_in_the_group() {
        assert_eq!(GlObject::Buffer(NativeBuffer(nz(1))).owning_context(), None);
        assert_eq!(
            GlObject::Program(NativeProgram(nz(1))).owning_context(),
            None,
        );
        assert_eq!(
            GlObject::Renderbuffer(NativeRenderbuffer(nz(1))).owning_context(),
            None,
        );
        assert_eq!(
            GlObject::Sampler(NativeSampler(nz(1))).owning_context(),
            None,
        );
        assert_eq!(GlObject::Shader(NativeShader(nz(1))).owning_context(), None);
        assert_eq!(
            GlObject::Texture(NativeTexture(nz(1))).owning_context(),
            None,
        );
    }
}
