//! Turns `MediaTransport1.Volume` reports (0-127) into dial turns, and parks the volume.
//!
//! Facts from `docs/phase0-findings.md`: one detent is 1/16 of the range (7 or 8 on the 0-127
//! scale), the sign is the direction, the buds clamp at both ends (so park at the midpoint outside
//! Volume mode), and our own park write echoes back once, 8-35 ms later.

use std::time::{Duration, Instant};

pub const PARK_VALUE: u16 = 64;
const ECHO_WINDOW: Duration = Duration::from_millis(400);
const ECHO_TOLERANCE: i32 = 2;
const STEP: f64 = 127.0 / 16.0;

#[derive(Debug, PartialEq, Eq)]
pub struct Decoded {
    /// Detents to report (signed), if any.
    pub steps: Option<i32>,
    /// Write this to `MediaTransport1.Volume` now.
    pub write: Option<u16>,
}

#[derive(Default)]
pub struct Encoder {
    parking: bool,
    last: Option<i32>,
    last_write: Option<(Instant, i32)>,
}

impl Encoder {
    /// The transport appeared or reconnected, and this is its volume.
    pub fn baseline(&mut self, volume: u16) {
        self.last = Some(volume as i32);
        self.last_write = None;
    }

    pub fn last_volume(&self) -> Option<u16> {
        self.last.map(|v| v.clamp(0, 127) as u16)
    }

    pub fn transport_lost(&mut self) {
        self.last = None;
        self.last_write = None;
    }

    /// Start or stop parking. Returns the write that starts it.
    pub fn set_parking(&mut self, on: bool, now: Instant) -> Option<u16> {
        if self.parking == on {
            return None;
        }
        self.parking = on;
        if on { Some(self.record_write(now)) } else { None }
    }

    pub fn report(&mut self, volume: u16, now: Instant) -> Decoded {
        let v = volume as i32;
        if self.parking {
            if let Some((at, written)) = self.last_write {
                if now.duration_since(at) < ECHO_WINDOW && (v - written).abs() <= ECHO_TOLERANCE {
                    self.last = Some(v);
                    return Decoded { steps: None, write: None };
                }
            }
        }
        let Some(prev) = self.last else {
            // No baseline (first report after a reconnect): can't tell direction.
            self.last = Some(v);
            return Decoded { steps: None, write: self.parking.then(|| self.record_write(now)) };
        };
        let delta = v - prev;
        self.last = Some(v);
        let magnitude = (delta.abs() as f64 / STEP).round() as i32;
        let steps = (magnitude > 0).then(|| magnitude * delta.signum());
        let write = (self.parking && steps.is_some()).then(|| self.record_write(now));
        Decoded { steps, write }
    }

    fn record_write(&mut self, now: Instant) -> u16 {
        self.last_write = Some((now, PARK_VALUE as i32));
        self.last = Some(PARK_VALUE as i32);
        PARK_VALUE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(base: Instant, n: u64) -> Instant {
        base + Duration::from_millis(n)
    }

    #[test]
    fn unparked_counts_detents_and_direction() {
        let t = Instant::now();
        let mut e = Encoder::default();
        e.baseline(64);
        assert_eq!(e.report(72, t).steps, Some(1));
        assert_eq!(e.report(65, ms(t, 300)).steps, Some(-1)); // 7 down
        assert_eq!(e.report(80, ms(t, 600)).steps, Some(2)); // merged two-step: +15
        assert_eq!(e.report(80, ms(t, 900)).steps, None);
    }

    #[test]
    fn parked_turn_writes_park_value_and_ignores_echo() {
        let t = Instant::now();
        let mut e = Encoder::default();
        e.baseline(64);
        assert_eq!(e.set_parking(true, t), Some(PARK_VALUE));
        // echo of the start-parking write
        assert_eq!(e.report(64, ms(t, 20)), Decoded { steps: None, write: None });
        // a turn right
        let d = e.report(71, ms(t, 500));
        assert_eq!(d, Decoded { steps: Some(1), write: Some(PARK_VALUE) });
        // its echo
        assert_eq!(e.report(64, ms(t, 520)), Decoded { steps: None, write: None });
        // a turn left, still decodes correctly after parking
        let d = e.report(56, ms(t, 900));
        assert_eq!(d, Decoded { steps: Some(-1), write: Some(PARK_VALUE) });
    }

    #[test]
    fn real_turn_inside_echo_window_is_not_swallowed() {
        let t = Instant::now();
        let mut e = Encoder::default();
        e.baseline(64);
        e.set_parking(true, t);
        let d = e.report(72, ms(t, 100)); // 8 away from the park value, not an echo
        assert_eq!(d.steps, Some(1));
    }

    #[test]
    fn leaving_park_keeps_baseline_at_park_value() {
        let t = Instant::now();
        let mut e = Encoder::default();
        e.baseline(100);
        e.set_parking(true, t);
        e.set_parking(false, ms(t, 50));
        assert_eq!(e.report(72, ms(t, 1000)).steps, Some(1));
    }

    #[test]
    fn first_report_without_baseline_gives_no_direction() {
        let t = Instant::now();
        let mut e = Encoder::default();
        assert_eq!(e.report(80, t).steps, None);
        assert_eq!(e.report(88, ms(t, 300)).steps, Some(1));
    }
}
