#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceLifecycleState {
    NoSurface,
    SurfacePending,
    SurfaceReady,
    Paused,
    Recovering,
    Lost,
}

#[derive(Debug, Clone)]
pub struct SurfaceSystem {
    state: SurfaceLifecycleState,
    size: Option<(u32, u32)>,
}

impl SurfaceSystem {
    pub fn new() -> Self {
        Self {
            state: SurfaceLifecycleState::NoSurface,
            size: None,
        }
    }

    pub fn state(&self) -> SurfaceLifecycleState {
        self.state
    }

    pub fn on_surface_available(&mut self, size: (u32, u32)) {
        self.size = Some(size);
        if self.state != SurfaceLifecycleState::Paused {
            self.state = SurfaceLifecycleState::SurfaceReady;
        }
    }

    pub fn on_surface_destroyed(&mut self) {
        self.size = None;
        self.state = match self.state {
            SurfaceLifecycleState::Paused => SurfaceLifecycleState::Lost,
            SurfaceLifecycleState::NoSurface | SurfaceLifecycleState::SurfacePending => {
                SurfaceLifecycleState::NoSurface
            }
            SurfaceLifecycleState::SurfaceReady
            | SurfaceLifecycleState::Recovering
            | SurfaceLifecycleState::Lost => SurfaceLifecycleState::Lost,
        };
    }

    pub fn on_pause(&mut self) {
        self.state = SurfaceLifecycleState::Paused;
    }

    pub fn on_resume(&mut self) {
        self.state = if self.size.is_some() {
            SurfaceLifecycleState::SurfaceReady
        } else {
            match self.state {
                SurfaceLifecycleState::Lost | SurfaceLifecycleState::Paused => {
                    SurfaceLifecycleState::Recovering
                }
                _ => SurfaceLifecycleState::SurfacePending,
            }
        };
    }

    pub fn can_present(&self) -> bool {
        self.state == SurfaceLifecycleState::SurfaceReady
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_available_while_paused_stays_paused_until_resume() {
        let mut system = SurfaceSystem::new();
        system.on_pause();
        system.on_surface_available((1080, 1920));

        assert_eq!(system.state(), SurfaceLifecycleState::Paused);
        assert!(!system.can_present());

        system.on_resume();

        assert_eq!(system.state(), SurfaceLifecycleState::SurfaceReady);
        assert!(system.can_present());
    }

    #[test]
    fn resume_without_any_surface_transitions_to_surface_pending() {
        let mut system = SurfaceSystem::new();

        system.on_resume();

        assert_eq!(system.state(), SurfaceLifecycleState::SurfacePending);
        assert!(!system.can_present());
    }

    #[test]
    fn resume_after_surface_destroyed_transitions_to_recovering() {
        let mut system = SurfaceSystem::new();
        system.on_surface_available((1080, 1920));
        system.on_surface_destroyed();

        system.on_resume();

        assert_eq!(system.state(), SurfaceLifecycleState::Recovering);
        assert!(!system.can_present());
    }

    #[test]
    fn resumes_into_recovering_after_surface_loss() {
        let mut system = SurfaceSystem::new();
        system.on_surface_available((1080, 1920));
        system.on_pause();
        system.on_surface_destroyed();
        system.on_resume();

        assert_eq!(system.state(), SurfaceLifecycleState::Recovering);
        assert!(!system.can_present());
    }

    #[test]
    fn transitions_back_to_surface_ready_after_recreate() {
        let mut system = SurfaceSystem::new();
        system.on_surface_available((1080, 1920));
        system.on_pause();
        system.on_surface_destroyed();
        system.on_resume();
        system.on_surface_available((1080, 1920));

        assert_eq!(system.state(), SurfaceLifecycleState::SurfaceReady);
        assert!(system.can_present());
    }
}
