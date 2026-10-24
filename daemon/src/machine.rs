//! The one state machine, ported from the macOS `DialModel`:
//!
//! ```text
//!   rest ──double press──▶ picking ──press──▶ inMode(m)
//!                            ▲  │                │
//!                            │  └─double press / idle─▶ back to where it came from
//!                            └──────double press─────┘
//! ```
//!
//! Pure logic: events and the clock go in, `Effect`s come out. The caller does the I/O.

use std::time::{Duration, Instant};

use crate::event::{DialEvent, ModeId};

pub const IDLE_DISMISS: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Rest,
    Picking,
    InMode(ModeId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Park the volume at the midpoint (true) or let it move freely (false).
    SetParking(bool),
    /// The wheel opened, moved or closed. `None` = closed.
    Wheel(Option<ModeId>),
    /// A mode was chosen, so it can set itself up.
    Activate(ModeId),
    Deactivate(ModeId),
    /// A detent inside a mode.
    ModeTurn(ModeId, i32),
    /// A press inside a mode.
    ModePress(ModeId),
}

pub struct Machine {
    phase: Phase,
    before_picking: Phase,
    selected: usize,
    modes: Vec<ModeId>,
    deadline: Option<Instant>,
}

impl Machine {
    pub fn new() -> Self {
        Machine {
            phase: Phase::Rest,
            before_picking: Phase::Rest,
            selected: 0,
            modes: ModeId::ALL.to_vec(),
            deadline: None,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn selected(&self) -> ModeId {
        self.modes[self.selected]
    }

    /// When `tick` next needs to run, if a timer is pending.
    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    pub fn describe(&self) -> String {
        match self.phase {
            Phase::Rest => "rest".into(),
            Phase::Picking => format!("picking ({})", self.selected().label()),
            Phase::InMode(m) => format!("in {}", m.label()),
        }
    }

    /// Parked everywhere except rest and Volume mode, where the dial is a volume knob.
    pub fn should_park(&self) -> bool {
        match self.phase {
            Phase::Rest => false,
            Phase::Picking => true,
            Phase::InMode(m) => m != ModeId::Volume,
        }
    }

    pub fn handle(&mut self, event: DialEvent, now: Instant) -> Vec<Effect> {
        let before = self.should_park();
        let mut fx = Vec::new();
        match (self.phase, event) {
            (Phase::Rest, DialEvent::Turn(_) | DialEvent::Press) => {
                // At rest the dial is plain earbuds: the volume already moved, and the
                // press already reached the media app.
            }
            (Phase::Rest | Phase::InMode(_), DialEvent::DoublePress) => self.begin_picking(now, &mut fx),

            (Phase::Picking, DialEvent::Turn(steps)) => {
                self.move_selection(steps);
                fx.push(Effect::Wheel(Some(self.selected())));
                self.deadline = Some(now + IDLE_DISMISS);
            }
            (Phase::Picking, DialEvent::Press) => self.choose(&mut fx),
            (Phase::Picking, DialEvent::DoublePress) => self.end_picking(&mut fx),

            (Phase::InMode(m), DialEvent::Turn(steps)) => {
                fx.push(Effect::ModeTurn(m, steps));
                // Apps keeps its fan open until the idle timeout.
                self.deadline = (m == ModeId::Apps).then_some(now + IDLE_DISMISS);
            }
            (Phase::InMode(m), DialEvent::Press) => {
                fx.push(Effect::ModePress(m));
                self.deadline = None;
            }
        }
        self.finish(before, &mut fx);
        fx
    }

    /// Run the idle timer. Call when `deadline()` has passed.
    pub fn tick(&mut self, now: Instant) -> Vec<Effect> {
        let mut fx = Vec::new();
        if self.deadline.is_some_and(|d| now >= d) {
            self.deadline = None;
            if self.phase == Phase::Picking {
                let before = self.should_park();
                self.end_picking(&mut fx);
                self.finish(before, &mut fx);
            }
        }
        fx
    }

    fn begin_picking(&mut self, now: Instant, fx: &mut Vec<Effect>) {
        self.before_picking = self.phase;
        let start = match self.phase {
            Phase::InMode(m) => m,
            _ => ModeId::Volume,
        };
        self.selected = self.modes.iter().position(|&m| m == start).unwrap_or(0);
        self.phase = Phase::Picking;
        self.deadline = Some(now + IDLE_DISMISS);
        fx.push(Effect::Wheel(Some(self.selected())));
    }

    fn choose(&mut self, fx: &mut Vec<Effect>) {
        let chosen = self.selected();
        if let Phase::InMode(old) = self.before_picking {
            if old != chosen {
                fx.push(Effect::Deactivate(old));
            }
        }
        self.phase = Phase::InMode(chosen);
        self.deadline = None;
        fx.push(Effect::Wheel(None));
        fx.push(Effect::Activate(chosen));
    }

    fn end_picking(&mut self, fx: &mut Vec<Effect>) {
        self.phase = self.before_picking;
        self.deadline = None;
        fx.push(Effect::Wheel(None));
    }

    fn move_selection(&mut self, steps: i32) {
        let n = self.modes.len() as i32;
        self.selected = (self.selected as i32 + steps).rem_euclid(n) as usize;
    }

    fn finish(&self, parked_before: bool, fx: &mut Vec<Effect>) {
        if self.should_park() != parked_before {
            fx.push(Effect::SetParking(self.should_park()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use DialEvent::*;

    fn run(m: &mut Machine, events: &[DialEvent]) -> Vec<Effect> {
        let t = Instant::now();
        events.iter().flat_map(|&e| m.handle(e, t)).collect()
    }

    #[test]
    fn double_press_opens_wheel_and_parks() {
        let mut m = Machine::new();
        let fx = run(&mut m, &[DoublePress]);
        assert_eq!(m.phase(), Phase::Picking);
        assert_eq!(fx, vec![Effect::Wheel(Some(ModeId::Volume)), Effect::SetParking(true)]);
    }

    #[test]
    fn turn_picks_and_press_chooses() {
        let mut m = Machine::new();
        run(&mut m, &[DoublePress, Turn(1)]);
        assert_eq!(m.selected(), ModeId::Brightness);
        let fx = run(&mut m, &[Press]);
        assert_eq!(m.phase(), Phase::InMode(ModeId::Brightness));
        assert_eq!(fx, vec![Effect::Wheel(None), Effect::Activate(ModeId::Brightness)]);
    }

    #[test]
    fn selection_wraps_both_ways() {
        let mut m = Machine::new();
        run(&mut m, &[DoublePress, Turn(-1)]);
        assert_eq!(m.selected(), ModeId::Slides);
        run(&mut m, &[Turn(1)]);
        assert_eq!(m.selected(), ModeId::Volume);
    }

    #[test]
    fn choosing_volume_unparks() {
        let mut m = Machine::new();
        let fx = run(&mut m, &[DoublePress, Press]);
        assert_eq!(m.phase(), Phase::InMode(ModeId::Volume));
        assert!(fx.contains(&Effect::SetParking(false)));
    }

    #[test]
    fn mode_turns_and_presses_reach_the_mode() {
        let mut m = Machine::new();
        run(&mut m, &[DoublePress, Turn(2), Press]);
        let fx = run(&mut m, &[Turn(-1), Press]);
        assert_eq!(fx, vec![Effect::ModeTurn(ModeId::Apps, -1), Effect::ModePress(ModeId::Apps)]);
    }

    #[test]
    fn double_press_in_mode_returns_there_when_cancelled() {
        let mut m = Machine::new();
        run(&mut m, &[DoublePress, Turn(1), Press]);
        run(&mut m, &[DoublePress]);
        assert_eq!(m.selected(), ModeId::Brightness);
        let fx = run(&mut m, &[DoublePress]);
        assert_eq!(m.phase(), Phase::InMode(ModeId::Brightness));
        assert_eq!(fx, vec![Effect::Wheel(None)]);
    }

    #[test]
    fn switching_mode_deactivates_the_old_one() {
        let mut m = Machine::new();
        run(&mut m, &[DoublePress, Turn(1), Press, DoublePress, Turn(1)]);
        let fx = run(&mut m, &[Press]);
        assert!(fx.contains(&Effect::Deactivate(ModeId::Brightness)));
        assert!(fx.contains(&Effect::Activate(ModeId::Apps)));
    }

    #[test]
    fn idle_timeout_closes_the_wheel() {
        let mut m = Machine::new();
        let t = Instant::now();
        m.handle(DoublePress, t);
        assert!(m.tick(t + Duration::from_secs(1)).is_empty());
        let fx = m.tick(t + IDLE_DISMISS);
        assert_eq!(m.phase(), Phase::Rest);
        assert_eq!(fx, vec![Effect::Wheel(None), Effect::SetParking(false)]);
    }

    #[test]
    fn rest_ignores_turn_and_press() {
        let mut m = Machine::new();
        assert!(run(&mut m, &[Turn(3), Press]).is_empty());
    }
}
