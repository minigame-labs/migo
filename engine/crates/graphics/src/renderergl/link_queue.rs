//! When to read a program's link status -- and why not immediately.
//!
//! `glLinkProgram` is asynchronous on every driver that matters; what actually
//! blocks is the first `glGetProgramiv(GL_LINK_STATUS)`. The handler used to
//! issue those back to back, so a bundle that links 40 programs at startup paid
//! 40 serialised driver compiles: each link finished before the next was even
//! submitted.
//!
//! Deferring the read lets every link in a batch overlap. The catch is that the
//! shader-binary cache save *needs* the status, and a program the content never
//! asks about would otherwise never be cached -- turning a warm start cold. So
//! the queue must always be drained, and the only question is when a drain is
//! allowed to stall:
//!
//! * The content asked (`getProgramParameter(LINK_STATUS)`, `getProgramInfoLog`)
//!   -- read now; that call's semantics are synchronous.
//! * `KHR_parallel_shader_compile` is available -- poll `COMPLETION_STATUS_KHR`
//!   and read only the ones already finished; the rest wait for a later frame.
//! * Neither -- read at the end of the batch, which still overlaps every link
//!   issued in it, and keeps the cache populated the same frame it always was.

/// `GL_COMPLETION_STATUS_KHR` from `KHR_parallel_shader_compile`. Not in
/// `glow`'s constant set, and it is the only token the extension adds that we
/// use.
pub(crate) const COMPLETION_STATUS_KHR: u32 = 0x91B1;

/// Why a drain is running.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DrainCause {
    /// Content asked for a status that cannot be answered any other way.
    ContentAsked,
    /// End of a GL batch: every link in it has been submitted.
    BatchEnd,
}

/// What to do with one pending program.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LinkProbe {
    /// Read `LINK_STATUS` now, accepting the stall, then save to the cache.
    ReadNow,
    /// The driver is still compiling and nobody is waiting -- try again later.
    LeavePending,
}

/// `completion_done` is `Some(_)` only when `KHR_parallel_shader_compile` is
/// available; `None` means the driver offers no way to ask without stalling.
pub(crate) fn probe(cause: DrainCause, completion_done: Option<bool>) -> LinkProbe {
    match (cause, completion_done) {
        // Somebody is blocked on the answer: the stall is the semantics.
        (DrainCause::ContentAsked, _) => LinkProbe::ReadNow,
        // No extension: reading at batch end still overlapped every link in the
        // batch, and it is what keeps the binary cache populated.
        (DrainCause::BatchEnd, None) => LinkProbe::ReadNow,
        (DrainCause::BatchEnd, Some(done)) => {
            if done {
                LinkProbe::ReadNow
            } else {
                LinkProbe::LeavePending
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_content_query_always_reads_even_mid_compile() {
        assert_eq!(
            probe(DrainCause::ContentAsked, Some(false)),
            LinkProbe::ReadNow,
            "getProgramParameter(LINK_STATUS) is synchronous by definition"
        );
        assert_eq!(probe(DrainCause::ContentAsked, None), LinkProbe::ReadNow);
    }

    /// Without the extension the queue must still drain every batch, or a
    /// bundle that never inspects link status would stop populating the shader
    /// binary cache and lose its warm start.
    #[test]
    fn without_the_extension_the_batch_end_drain_reads_everything() {
        assert_eq!(probe(DrainCause::BatchEnd, None), LinkProbe::ReadNow);
    }

    #[test]
    fn with_the_extension_only_finished_links_are_read() {
        assert_eq!(probe(DrainCause::BatchEnd, Some(true)), LinkProbe::ReadNow);
        assert_eq!(
            probe(DrainCause::BatchEnd, Some(false)),
            LinkProbe::LeavePending
        );
    }

    #[test]
    fn the_completion_token_is_the_one_the_extension_defines() {
        assert_eq!(COMPLETION_STATUS_KHR, 0x91B1);
    }
}

/// Source-shaped guards.
///
/// The behaviour these protect is a *timing* property -- "the status read does
/// not happen here" -- which no host test can observe without a driver, and
/// which is exactly the kind of thing a later edit re-introduces by accident
/// (the obvious way to write the arm is `link` then `get_link_status`). So the
/// guard reads the source and fails on the shape.
#[cfg(test)]
mod source_guards {
    const HANDLER: &str = include_str!("handler.rs");
    const RENDER_THREAD: &str = include_str!("../render_thread.rs");

    fn link_program_arm() -> &'static str {
        let start = HANDLER
            .find("GLCmd::LinkProgram { program_id } => {")
            .expect("the LinkProgram arm must exist");
        let rest = &HANDLER[start..];
        let end = rest
            .find("GLCmd::BindAttribLocation")
            .expect("the arm after LinkProgram must exist");
        &rest[..end]
    }

    #[test]
    fn linking_does_not_read_the_status_it_just_issued() {
        let arm = link_program_arm();
        assert!(
            arm.contains("gl.link_program(ph)"),
            "the arm must still issue the link"
        );
        let after_link = &arm[arm.find("gl.link_program(ph)").unwrap()..];
        assert!(
            !after_link.contains("get_program_link_status"),
            "reading LINK_STATUS right after linking serialises the driver's \
             compiles -- that read belongs in the queue drain"
        );
        assert!(
            arm.contains("mark_link_pending"),
            "a deferred link must be queued, or its binary is never cached"
        );
    }

    /// The cache-hit branch reads the status on purpose: `glProgramBinary` is
    /// allowed to fail, and the fallback to a real link depends on the answer.
    #[test]
    fn the_cached_binary_branch_still_checks_that_it_took() {
        let arm = link_program_arm();
        let hit = &arm[..arm.find("gl.link_program(ph)").unwrap()];
        assert!(
            hit.contains("get_program_link_status"),
            "a rejected program binary must still fall back to linking"
        );
    }

    #[test]
    fn every_batch_drains_the_queue() {
        assert!(
            RENDER_THREAD.contains("drain_pending_links(cm, gl, DrainCause::BatchEnd)"),
            "without a batch-end drain a bundle that never inspects link status \
             would stop populating the shader binary cache"
        );
    }

    #[test]
    fn a_deleted_program_leaves_the_queue() {
        let start = HANDLER
            .find("GLCmd::DeleteProgram { program_id } => {")
            .expect("the DeleteProgram arm must exist");
        let arm = &HANDLER[start..start + 600];
        assert!(
            arm.contains("forget_pending_link"),
            "a drain must never look up a program whose handle is gone"
        );
    }
}
