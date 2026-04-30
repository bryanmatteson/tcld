#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeAction {
    StepInto,
    StepOver,
    StepOut,
    Continue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepMode {
    StepInto,
    StepOver { target_level: i32 },
    StepOut { target_level: i32 },
    Continue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    Step,
    Breakpoint,
    Pause,
}

impl StopReason {
    pub fn as_str(self) -> &'static str {
        match self {
            StopReason::Step => "step",
            StopReason::Breakpoint => "breakpoint",
            StopReason::Pause => "pause",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RunControlCore {
    step_mode: StepMode,
    pause_requested: bool,
}

impl Default for RunControlCore {
    fn default() -> Self {
        Self::new_start_step_into()
    }
}

impl RunControlCore {
    pub fn new_start_step_into() -> Self {
        Self {
            step_mode: StepMode::StepInto,
            pause_requested: false,
        }
    }

    pub fn request_pause(&mut self) {
        self.pause_requested = true;
    }

    pub fn on_resume(&mut self, action: ResumeAction, current_level: i32) {
        self.pause_requested = false;
        self.step_mode = match action {
            ResumeAction::StepInto => StepMode::StepInto,
            ResumeAction::StepOver => StepMode::StepOver {
                target_level: current_level,
            },
            ResumeAction::StepOut => StepMode::StepOut {
                target_level: current_level,
            },
            ResumeAction::Continue => StepMode::Continue,
        };
    }

    pub fn decide_stop(&mut self, level: i32, breakpoint_hit: bool) -> Option<StopReason> {
        if self.pause_requested {
            self.pause_requested = false;
            return Some(StopReason::Pause);
        }

        if breakpoint_hit {
            return Some(StopReason::Breakpoint);
        }

        match self.step_mode {
            StepMode::StepInto => Some(StopReason::Step),
            StepMode::StepOver { target_level } if level <= target_level => Some(StopReason::Step),
            StepMode::StepOut { target_level } if level < target_level => Some(StopReason::Step),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ResumeAction, RunControlCore, StopReason};

    #[test]
    fn initial_step_into_stops() {
        let mut core = RunControlCore::new_start_step_into();
        assert_eq!(core.decide_stop(1, false), Some(StopReason::Step));
    }

    #[test]
    fn step_over_stops_at_same_or_shallower_level() {
        let mut core = RunControlCore::new_start_step_into();
        core.on_resume(ResumeAction::StepOver, 3);
        assert_eq!(core.decide_stop(4, false), None);
        assert_eq!(core.decide_stop(3, false), Some(StopReason::Step));
        core.on_resume(ResumeAction::StepOver, 3);
        assert_eq!(core.decide_stop(2, false), Some(StopReason::Step));
    }

    #[test]
    fn pause_request_overrides_other_modes_once() {
        let mut core = RunControlCore::new_start_step_into();
        core.on_resume(ResumeAction::Continue, 0);
        core.request_pause();
        assert_eq!(core.decide_stop(2, false), Some(StopReason::Pause));
        assert_eq!(core.decide_stop(2, false), None);
    }

    #[test]
    fn breakpoint_hit_wins_when_no_pause_request() {
        let mut core = RunControlCore::new_start_step_into();
        core.on_resume(ResumeAction::Continue, 0);
        assert_eq!(core.decide_stop(2, true), Some(StopReason::Breakpoint));
    }
}
