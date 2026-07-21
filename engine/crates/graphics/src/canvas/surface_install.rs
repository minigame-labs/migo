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
        InstallPolicy::Skip
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
                    force,
                ),
                InstallPolicy::FullRecreate,
            );
        }
    }
}
