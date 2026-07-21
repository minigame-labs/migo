use std::fmt;

use shared::surface::{SurfaceGeneration, SurfaceLease};

/// The Host's unique logical ownership slot for one Surface generation.
///
/// This owner is deliberately not `Clone`; only read-only `SurfaceLease`
/// values may cross the Host/render handoff.
pub(crate) struct SurfaceAttachment {
    lease: SurfaceLease,
}

/// Generation-aware Host Surface state.
pub(crate) struct SurfaceAttachmentSlot {
    current: Option<SurfaceAttachment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SurfaceTransitionError {
    /// The candidate is retired or older than the attachment already observed.
    StaleGeneration,
    /// A newer candidate arrived while a different generation remains live.
    ConflictingLiveGeneration,
}

impl fmt::Display for SurfaceTransitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleGeneration => formatter.write_str("stale Surface generation"),
            Self::ConflictingLiveGeneration => {
                formatter.write_str("conflicting live Surface generation")
            }
        }
    }
}

impl std::error::Error for SurfaceTransitionError {}

impl SurfaceAttachmentSlot {
    pub(crate) const fn empty() -> Self {
        Self { current: None }
    }

    pub(crate) fn from_initial(lease: SurfaceLease) -> Self {
        debug_assert!(lease.is_live(), "initial Surface lease must be live");
        Self {
            current: Some(SurfaceAttachment { lease }),
        }
    }

    #[inline]
    pub(crate) fn generation(&self) -> Option<SurfaceGeneration> {
        self.current
            .as_ref()
            .map(|attachment| attachment.lease.generation())
    }

    #[inline]
    pub(crate) fn has_live_surface(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|attachment| attachment.lease.is_live())
    }

    /// Returns a cloneable read-only lease only when the exact generation is
    /// still live. Used for the explicit restore handoff.
    pub(crate) fn live_lease(&self) -> Option<SurfaceLease> {
        self.current
            .as_ref()
            .filter(|attachment| attachment.lease.is_live())
            .map(|attachment| attachment.lease.clone())
    }

    /// Validate a candidate without mutating the current owner.
    pub(crate) fn prepare(&self, candidate: &SurfaceLease) -> Result<(), SurfaceTransitionError> {
        if !candidate.is_live() {
            return Err(SurfaceTransitionError::StaleGeneration);
        }

        let Some(current) = self.current.as_ref() else {
            return Ok(());
        };

        let current_generation = current.lease.generation();
        let candidate_generation = candidate.generation();
        if candidate_generation < current_generation {
            return Err(SurfaceTransitionError::StaleGeneration);
        }
        if candidate_generation > current_generation && current.lease.is_live() {
            return Err(SurfaceTransitionError::ConflictingLiveGeneration);
        }

        Ok(())
    }

    /// Commit a candidate after a successful backend recreate.
    pub(crate) fn commit(&mut self, candidate: SurfaceLease) -> Result<(), SurfaceTransitionError> {
        self.prepare(&candidate)?;
        self.current = Some(SurfaceAttachment { lease: candidate });
        Ok(())
    }

    /// Clear only the exact generation named by a lifecycle command.
    pub(crate) fn detach(&mut self, generation: SurfaceGeneration) -> bool {
        if self.generation() != Some(generation) {
            return false;
        }
        self.current = None;
        true
    }
}

impl Default for SurfaceAttachmentSlot {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use shared::surface::{
        Surface, SurfaceGenerationGate, SurfaceLease, SurfaceLivenessToken, SurfaceRef,
    };
    use std::sync::Arc;

    use super::{SurfaceAttachmentSlot, SurfaceTransitionError};

    #[derive(Debug)]
    struct TestSurface(u32);

    impl Surface for TestSurface {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        fn size(&self) -> (u32, u32) {
            (self.0, self.0)
        }
    }

    fn lease(token: SurfaceLivenessToken, marker: u32) -> SurfaceLease {
        let surface: SurfaceRef = Arc::new(TestSurface(marker));
        SurfaceLease::new(surface, token)
    }

    #[test]
    fn empty_slot_accepts_and_commits_a_live_lease() {
        let gate = Arc::new(SurfaceGenerationGate::new());
        let candidate = lease(gate.attach_or_update().unwrap(), 1);
        let generation = candidate.generation();
        let mut slot = SurfaceAttachmentSlot::empty();

        assert_eq!(slot.generation(), None);
        assert!(!slot.has_live_surface());
        assert_eq!(slot.prepare(&candidate), Ok(()));
        assert_eq!(slot.generation(), None);

        slot.commit(candidate).unwrap();
        assert_eq!(slot.generation(), Some(generation));
        assert!(slot.has_live_surface());
    }

    #[test]
    fn same_generation_update_preserves_the_fast_path_generation() {
        let gate = Arc::new(SurfaceGenerationGate::new());
        let first = lease(gate.attach_or_update().unwrap(), 1);
        let same_generation = lease(gate.attach_or_update().unwrap(), 2);
        let generation = first.generation();
        let mut slot = SurfaceAttachmentSlot::from_initial(first);

        assert_eq!(slot.prepare(&same_generation), Ok(()));
        slot.commit(same_generation).unwrap();

        assert_eq!(slot.generation(), Some(generation));
        assert!(slot.has_live_surface());
    }

    #[test]
    fn retired_attachment_can_be_replaced_but_old_update_stays_stale() {
        let gate = Arc::new(SurfaceGenerationGate::new());
        let first = lease(gate.attach_or_update().unwrap(), 1);
        let stale_retry = first.clone();
        let mut slot = SurfaceAttachmentSlot::from_initial(first);

        gate.retire_current().unwrap();
        assert!(!slot.has_live_surface());
        let second = lease(gate.attach_or_update().unwrap(), 2);
        let second_generation = second.generation();

        assert_eq!(slot.prepare(&second), Ok(()));
        slot.commit(second).unwrap();
        assert_eq!(slot.generation(), Some(second_generation));
        assert_eq!(
            slot.prepare(&stale_retry),
            Err(SurfaceTransitionError::StaleGeneration)
        );
    }

    #[test]
    fn different_simultaneous_live_generation_is_rejected() {
        let first_gate = Arc::new(SurfaceGenerationGate::new());
        let first = lease(first_gate.attach_or_update().unwrap(), 1);
        let slot = SurfaceAttachmentSlot::from_initial(first);

        let foreign_gate = Arc::new(SurfaceGenerationGate::new());
        foreign_gate.attach_or_update().unwrap();
        foreign_gate.retire_current().unwrap();
        let foreign_generation_two = lease(foreign_gate.attach_or_update().unwrap(), 2);

        assert_eq!(
            slot.prepare(&foreign_generation_two),
            Err(SurfaceTransitionError::ConflictingLiveGeneration)
        );
    }

    #[test]
    fn delayed_destroy_cannot_clear_a_newer_attachment() {
        let gate = Arc::new(SurfaceGenerationGate::new());
        let first = lease(gate.attach_or_update().unwrap(), 1);
        let generation_one = first.generation();
        let mut slot = SurfaceAttachmentSlot::from_initial(first);

        gate.retire_current().unwrap();
        let second = lease(gate.attach_or_update().unwrap(), 2);
        let generation_two = second.generation();
        slot.commit(second).unwrap();

        assert!(!slot.detach(generation_one));
        assert_eq!(slot.generation(), Some(generation_two));
        assert!(slot.has_live_surface());
        assert!(slot.detach(generation_two));
        assert_eq!(slot.generation(), None);
    }

    #[test]
    fn failed_render_response_leaves_the_previous_attachment_unchanged() {
        let gate = Arc::new(SurfaceGenerationGate::new());
        let first = lease(gate.attach_or_update().unwrap(), 1);
        let generation = first.generation();
        let candidate = lease(gate.attach_or_update().unwrap(), 2);
        let slot = SurfaceAttachmentSlot::from_initial(first);

        assert_eq!(slot.prepare(&candidate), Ok(()));
        // A failed render response never reaches `commit`.
        assert_eq!(slot.generation(), Some(generation));
        assert!(slot.has_live_surface());
    }
}
