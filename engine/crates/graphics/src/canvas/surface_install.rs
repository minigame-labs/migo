use crate::surface_binding::RecreateKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InstallPolicy {
    Skip,
    FastResize,
    FullRecreate,
}

/// Pure cold-path policy. Reuse is permitted only when backend-specific native
/// equivalence has already been proven and no generation/force boundary
/// requires a fresh EGLSurface.
pub(crate) fn classify_surface_install(
    kind: RecreateKind,
    native_equivalent: bool,
    installed_size: Option<(u32, u32)>,
    requested_size: Option<(u32, u32)>,
    has_onscreen_2d: bool,
    owes_2d_restore: bool,
    force_recreate: bool,
) -> InstallPolicy {
    if force_recreate
        || kind != RecreateKind::SameGeneration
        || !native_equivalent
        || installed_size.is_none()
        || requested_size.is_none()
    {
        return InstallPolicy::FullRecreate;
    }

    if installed_size == requested_size {
        // Skipping is only safe when the install owes nothing. A canvas whose
        // 2D context was torn down by an earlier event still looks
        // same-window/same-size, and skipping leaves it without a context for
        // the rest of the session -- the content holds the object it got from
        // `getContext('2d')` and never asks again, so nothing would rebuild it.
        //
        // `owes_2d_restore` rather than `!has_onscreen_2d`: a WebGL-only canvas
        // has no 2D context and never will, and must keep taking this fast
        // path. The distinction is "never had one" versus "had one and lost
        // it", which is exactly what the absent-context check cannot make.
        if owes_2d_restore {
            InstallPolicy::FullRecreate
        } else {
            InstallPolicy::Skip
        }
    } else if has_onscreen_2d {
        InstallPolicy::FastResize
    } else {
        InstallPolicy::FullRecreate
    }
}

#[cfg(test)]
mod tests {
    use crate::surface_binding::RecreateKind;

    use super::{InstallPolicy, classify_surface_install};

    #[test]
    fn surface_install_policy_preserves_fast_paths_without_cross_generation_reuse() {
        let old_size = (1080, 1920);
        let new_size = (1920, 1080);

        assert_eq!(
            classify_surface_install(
                RecreateKind::SameGeneration,
                true,
                Some(old_size),
                Some(old_size),
                true,
                false,
                false,
            ),
            InstallPolicy::Skip,
        );
        assert_eq!(
            classify_surface_install(
                RecreateKind::SameGeneration,
                true,
                Some(old_size),
                Some(new_size),
                true,
                false,
                false,
            ),
            InstallPolicy::FastResize,
        );
        assert_eq!(
            classify_surface_install(
                RecreateKind::NewGeneration,
                true,
                Some(old_size),
                Some(old_size),
                true,
                false,
                false,
            ),
            InstallPolicy::FullRecreate,
        );
    }

    #[test]
    fn surface_install_policy_fails_to_full_recreate_when_reuse_is_not_proven() {
        let size = (1080, 1920);
        for (kind, equivalent, has_2d, force) in [
            (RecreateKind::Initial, false, false, false),
            (RecreateKind::SameGeneration, false, true, false),
            (RecreateKind::SameGeneration, true, false, false),
            (RecreateKind::SameGeneration, true, true, true),
        ] {
            assert_eq!(
                classify_surface_install(
                    kind,
                    equivalent,
                    Some(size),
                    Some((size.0 + 1, size.1)),
                    has_2d,
                    false,
                    force,
                ),
                InstallPolicy::FullRecreate,
            );
        }
    }

    /// An outstanding 2D restore forbids the same-size fast path.
    ///
    /// A canvas whose 2D context was torn down by an earlier event still looks
    /// same-window and same-size, so every other input says "skip". Skipping
    /// leaves it with no context for the rest of the session: the content holds
    /// the object `getContext('2d')` gave it and never asks again, so nothing
    /// would ever rebuild it. Measured cost when this went wrong on a real
    /// surface path: ~900k `2d context not found` per 8s, frozen picture, game
    /// still running at 60fps.
    #[test]
    fn an_outstanding_2d_restore_forbids_the_same_size_fast_path() {
        let size = (1080, 1920);
        assert_eq!(
            classify_surface_install(
                RecreateKind::SameGeneration,
                true,
                Some(size),
                Some(size),
                false,
                true,
                false,
            ),
            InstallPolicy::FullRecreate,
            "an install that owes a 2D restore must not skip",
        );
    }

    /// ...and a canvas that never had a 2D context still takes it.
    ///
    /// This is the distinction the absent-context check cannot make: "never had
    /// one" (WebGL-only content, the common case) versus "had one and lost it".
    /// Keying the skip on `!has_onscreen_2d` instead of the outstanding
    /// obligation would put every WebGL resume through a full recreate.
    #[test]
    fn a_canvas_that_never_had_a_2d_context_still_takes_the_fast_path() {
        let size = (1080, 1920);
        assert_eq!(
            classify_surface_install(
                RecreateKind::SameGeneration,
                true,
                Some(size),
                Some(size),
                false,
                false,
                false,
            ),
            InstallPolicy::Skip,
            "WebGL-only content owes nothing and must keep the fast path",
        );
    }
}
