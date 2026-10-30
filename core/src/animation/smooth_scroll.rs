//! Ease a scroll offset toward a target, the way a browser animates a mouse wheel.

// The easing curve, the durations and the velocity-preserving retargeting
// follow Chromium's wheel scroll animation
// (`cc/animation/scroll_offset_animation_curve.{h,cc}`, BSD 3-Clause:
// <https://chromium.googlesource.com/chromium/src/+/main/LICENSE>), as ported
// to iced upstream in iced-rs/iced#3479.
use crate::Vector;
use crate::time::{Duration, Instant};

use std::cell::Cell;

thread_local! {
    static INSTANT: Cell<bool> = const { Cell::new(false) };
}

/// A smooth scroll: each wheel notch moves a target, and the offset eases
/// toward it, keeping its speed when another notch moves the target again.
///
/// Offsets are absolute and run from zero to a maximum per axis. The owner
/// keeps the current offset; [`SmoothScroll`] only says where it should be.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SmoothScroll {
    run: Option<Run>,
}

/// One segment of the animation, from `start` at rest or at speed toward
/// `destination`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Run {
    destination: Vector,
    start: Vector,
    started: Instant,
    /// In seconds.
    duration: f32,
    /// The curve's initial slope: 0 starts from rest.
    slope: f32,
}

impl SmoothScroll {
    /// The distance one wheel notch, or one line, scrolls, in logical pixels.
    pub const LINE: f32 = 120.0;

    /// The ease-in-out cubic bezier's control points, Chromium's for wheel scrolls.
    const BEZIER_X1: f32 = 0.42;
    const BEZIER_X2: f32 = 0.58;

    /// Durations are counted in frames of a nominal 60 Hz display.
    const FRAMES_PER_SECOND: f32 = 60.0;

    /// A wheel event is handled between frames; starting the animation one
    /// frame early makes the first drawn frame already move.
    const FRAME_DELAY: f32 = 1.0 / Self::FRAMES_PER_SECOND;

    /// Distances at which the duration ramp starts and ends, in pixels.
    const RAMP_START: f32 = Self::LINE;
    const RAMP_END: f32 = 480.0;

    /// The longest and shortest durations, in frames: a short scroll gets the
    /// softer, longer animation.
    const DURATION_MAX: f32 = 12.0;
    const DURATION_MIN: f32 = 6.0;

    /// Bounds a retargeted run to this many times the time the new distance
    /// takes at the current speed, so a small delta at speed doesn't rubber band.
    const RETARGET_VELOCITY_BOUND: f32 = 2.5;

    /// The clamp for a retargeted run's initial slope.
    const SLOPE_CLAMP: f32 = 1000.0;

    /// Creates a [`SmoothScroll`] at rest.
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes every [`SmoothScroll`] on this thread land on its target at once.
    ///
    /// Headless harnesses draw frames on the wall clock, so an eased scroll
    /// would be captured part of the way there.
    pub fn set_instant(instant: bool) {
        INSTANT.with(|cell| cell.set(instant));
    }

    fn is_instant() -> bool {
        INSTANT.with(Cell::get)
    }

    /// Whether an animation is running.
    pub fn is_animating(&self) -> bool {
        self.run.is_some()
    }

    /// Where the running animation ends, if one is running.
    pub fn destination(&self) -> Option<Vector> {
        self.run.map(|run| run.destination)
    }

    /// Moves the target by `delta`, from the pending target if there is one,
    /// so quick notches add up instead of losing the distance not yet covered.
    ///
    /// Offsets are clamped to `0..=max`. Returns the offset to show now: the
    /// `position` given while an animation runs, the target when it lands at once.
    pub fn scroll_by(
        &mut self,
        position: Vector,
        delta: Vector,
        max: Vector,
        now: Instant,
    ) -> Vector {
        let base = self.destination().unwrap_or(position);

        self.scroll_to(position, clamp(base + delta, max), now)
    }

    /// Eases from `position` to `destination`.
    ///
    /// A running animation is retargeted from where it is and how fast it
    /// moves, so scrolling flows on instead of restarting. Returns the offset to
    /// show now, as [`scroll_by`](Self::scroll_by) does.
    pub fn scroll_to(&mut self, position: Vector, destination: Vector, now: Instant) -> Vector {
        if Self::is_instant() || destination == position {
            self.run = None;
            return destination;
        }

        let now = now
            .checked_sub(Duration::from_secs_f32(Self::FRAME_DELAY))
            .unwrap_or(now);

        match self.run {
            None => {
                let distance = (destination - position)
                    .x
                    .abs()
                    .max((destination - position).y.abs());

                self.run = Some(Run {
                    destination,
                    start: position,
                    started: now,
                    duration: duration(distance),
                    slope: 0.0,
                });
            }
            Some(run) if run.destination == destination => {}
            Some(_) => self.retarget(destination, now),
        }

        position
    }

    /// Advances the animation to `now`, returning the offset to show, or
    /// `None` when nothing runs. The last frame lands on the target exactly
    /// and ends the animation.
    ///
    /// `max` is checked again: content that shrank under the animation
    /// retargets it to the new end.
    pub fn step(&mut self, now: Instant, max: Vector) -> Option<Vector> {
        let run = self.run?;
        let destination = clamp(run.destination, max);

        if destination != run.destination {
            self.retarget(destination, now);
        }

        let Some(run) = self.run else {
            return Some(destination);
        };

        let progress = run.progress(now);

        if progress >= 1.0 {
            self.run = None;
            return Some(run.destination);
        }

        // A retargeted run can overshoot its end on the way in.
        Some(clamp(run.position_at(progress), max))
    }

    /// Moves the whole animation by `offset`, for when the offsets it eases
    /// between move with the content, such as content added above them.
    pub fn shift(&mut self, offset: Vector) {
        if let Some(run) = &mut self.run {
            run.start += offset;
            run.destination += offset;
        }
    }

    /// Stops the animation where it is; the owner keeps its offset.
    pub fn cancel(&mut self) {
        self.run = None;
    }

    /// Restarts the running animation toward `destination` from its position
    /// and speed at `now`.
    fn retarget(&mut self, destination: Vector, now: Instant) {
        let Some(run) = self.run else {
            return;
        };

        let progress = run.progress(now);
        let start = run.position_at(progress);
        let velocity = run.velocity_at(progress);
        let distance = longest(destination - start);

        let mut duration = duration(distance.abs());

        if velocity.abs() > 0.01 {
            // A bound with the wrong sign means the new delta goes against the
            // motion, and does not apply.
            let bound = Self::RETARGET_VELOCITY_BOUND * distance / velocity;

            if bound > 0.0 {
                duration = duration.min(bound);
            }
        }

        if distance.abs() < 0.01 || duration < 0.01 {
            self.run = None;
            return;
        }

        self.run = Some(Run {
            destination,
            start,
            started: now,
            duration,
            slope: (velocity * duration / distance).clamp(-Self::SLOPE_CLAMP, Self::SLOPE_CLAMP),
        });
    }
}

impl Run {
    fn progress(&self, now: Instant) -> f32 {
        let elapsed = now.saturating_duration_since(self.started).as_secs_f32();

        (elapsed / self.duration).clamp(0.0, 1.0)
    }

    fn position_at(&self, progress: f32) -> Vector {
        self.start + (self.destination - self.start) * ease(progress, self.slope)
    }

    /// The speed along the longest axis, in pixels per second.
    fn velocity_at(&self, progress: f32) -> f32 {
        if progress >= 1.0 {
            return 0.0;
        }

        slope_at(progress, self.slope) * longest(self.destination - self.start) / self.duration
    }
}

/// The duration of a run covering `distance` pixels, in seconds: a short
/// scroll gets the longer, softer animation.
fn duration(distance: f32) -> f32 {
    let slope = (SmoothScroll::DURATION_MIN - SmoothScroll::DURATION_MAX)
        / (SmoothScroll::RAMP_END - SmoothScroll::RAMP_START);
    let offset = SmoothScroll::DURATION_MAX - SmoothScroll::RAMP_START * slope;

    (offset + distance * slope).clamp(SmoothScroll::DURATION_MIN, SmoothScroll::DURATION_MAX)
        / SmoothScroll::FRAMES_PER_SECOND
}

/// The eased progress at time `progress` (`0..=1`), for a curve whose first
/// control point is lifted by `slope` (0 is the plain ease-in-out).
fn ease(progress: f32, slope: f32) -> f32 {
    if progress <= 0.0 {
        return 0.0;
    }

    if progress >= 1.0 {
        return 1.0;
    }

    let s = solve(progress);
    let y1 = SmoothScroll::BEZIER_X1 * slope;
    let os = 1.0 - s;

    3.0 * y1 * s * os * os + 3.0 * s * s * os + s * s * s
}

/// The curve's slope (dy/dx) at time `progress`.
fn slope_at(progress: f32, slope: f32) -> f32 {
    let s = if progress <= 0.0 {
        0.0
    } else if progress >= 1.0 {
        1.0
    } else {
        solve(progress)
    };

    let x1 = SmoothScroll::BEZIER_X1;
    let x2 = SmoothScroll::BEZIER_X2;
    let y1 = x1 * slope;
    let os = 1.0 - s;

    let dx = 3.0 * x1 * os * (1.0 - 3.0 * s) + 3.0 * x2 * s * (2.0 - 3.0 * s) + 3.0 * s * s;
    let dy = 3.0 * y1 * os * (1.0 - 3.0 * s) + 3.0 * s * (2.0 - 3.0 * s) + 3.0 * s * s;

    dy / dx
}

/// Solves `x(s) = progress` for the bezier's parameter `s`. The x axis only
/// grows for these control points, so bisection always converges.
fn solve(progress: f32) -> f32 {
    let x1 = SmoothScroll::BEZIER_X1;
    let x2 = SmoothScroll::BEZIER_X2;

    let x = |s: f32| {
        let os = 1.0 - s;
        3.0 * x1 * s * os * os + 3.0 * x2 * s * s * os + s * s * s
    };

    let mut low = 0.0;
    let mut high = 1.0;

    for _ in 0..40 {
        let mid = (low + high) / 2.0;

        if x(mid) < progress {
            low = mid;
        } else {
            high = mid;
        }
    }

    (low + high) / 2.0
}

/// The signed component with the largest magnitude.
fn longest(vector: Vector) -> f32 {
    if vector.x.abs() > vector.y.abs() {
        vector.x
    } else {
        vector.y
    }
}

fn clamp(offset: Vector, max: Vector) -> Vector {
    Vector::new(
        offset.x.clamp(0.0, max.x.max(0.0)),
        offset.y.clamp(0.0, max.y.max(0.0)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX: Vector = Vector::new(0.0, 10_000.0);

    fn ms(milliseconds: u64) -> Duration {
        Duration::from_millis(milliseconds)
    }

    fn down(lines: f32) -> Vector {
        Vector::new(0.0, lines * SmoothScroll::LINE)
    }

    /// Steps 60 Hz frames from `from` until the animation ends, returning
    /// each frame's offset.
    fn frames(scroll: &mut SmoothScroll, from: Instant) -> Vec<f32> {
        let mut offsets = Vec::new();
        let mut now = from;

        while let Some(offset) = scroll.step(now, MAX) {
            offsets.push(offset.y);
            now += ms(16);

            assert!(offsets.len() < 100, "the animation never ends");
        }

        offsets
    }

    #[test]
    fn a_notch_eases_one_line_and_lands_on_it() {
        let mut scroll = SmoothScroll::new();
        let now = Instant::now();

        let shown = scroll.scroll_by(Vector::ZERO, down(1.0), MAX, now);
        assert_eq!(shown, Vector::ZERO, "nothing moves before a frame is drawn");

        let offsets = frames(&mut scroll, now);

        assert!(offsets[0] > 0.0, "the first frame already moves");
        assert!(
            offsets[0] < 30.0,
            "the first frame eases in, got {}",
            offsets[0]
        );
        assert!(
            offsets.windows(2).all(|pair| pair[0] <= pair[1]),
            "{offsets:?}"
        );
        assert_eq!(offsets.last(), Some(&SmoothScroll::LINE));
        assert!(!scroll.is_animating());
        // One notch: 12 frames, the longest run.
        assert!(
            (10..=13).contains(&offsets.len()),
            "{} frames",
            offsets.len()
        );
    }

    #[test]
    fn the_curve_eases_in_and_out() {
        assert_eq!(ease(0.0, 0.0), 0.0);
        assert_eq!(ease(1.0, 0.0), 1.0);
        assert!((ease(0.5, 0.0) - 0.5).abs() < 1e-4);
        assert!(ease(0.1, 0.0) < 0.1, "slow at the start");
        assert!(ease(0.9, 0.0) > 0.9, "slow at the end");
        assert!(slope_at(0.0, 0.0).abs() < 1e-4, "starts from rest");
        assert!(slope_at(1.0, 0.0).abs() < 1e-4, "ends at rest");
        assert!(
            (slope_at(0.0, 2.0) - 2.0).abs() < 1e-3,
            "a lifted curve starts at speed"
        );
    }

    #[test]
    fn longer_scrolls_animate_quicker() {
        assert!((duration(SmoothScroll::LINE) - 0.2).abs() < 1e-6);
        assert!((duration(300.0) - 0.15).abs() < 1e-6);
        assert!((duration(480.0) - 0.1).abs() < 1e-6);
        assert!((duration(5_000.0) - 0.1).abs() < 1e-6);
        assert!((duration(10.0) - 0.2).abs() < 1e-6);
    }

    #[test]
    fn another_notch_adds_up_and_keeps_the_speed() {
        let mut scroll = SmoothScroll::new();
        let start = Instant::now();

        let _ = scroll.scroll_by(Vector::ZERO, down(1.0), MAX, start);
        let mid = start + ms(80);
        let before = scroll.step(mid, MAX).expect("running").y;
        let speed = scroll
            .run
            .expect("running")
            .velocity_at(scroll.run.unwrap().progress(mid));

        let _ = scroll.scroll_by(Vector::new(0.0, before), down(1.0), MAX, mid);
        assert_eq!(
            scroll.destination(),
            Some(down(2.0)),
            "the second notch adds to the first"
        );

        // The new run starts one frame back, where the old one was, at its speed.
        let run = scroll.run.expect("running");
        let restarted = run.progress(mid);
        let after = run.position_at(restarted).y;
        let new_speed = run.velocity_at(restarted);

        assert!(after >= before, "no jump back: {before} → {after}");
        assert!(after - before < 20.0, "no jump ahead: {before} → {after}");
        assert!(
            new_speed > 0.5 * speed,
            "keeps going: {speed} → {new_speed}"
        );

        let offsets = frames(&mut scroll, mid + ms(16));
        assert!(
            offsets.windows(2).all(|pair| pair[0] <= pair[1]),
            "{offsets:?}"
        );
        assert_eq!(offsets.last(), Some(&(2.0 * SmoothScroll::LINE)));
    }

    #[test]
    fn a_notch_against_the_motion_turns_back() {
        let mut scroll = SmoothScroll::new();
        let start = Instant::now();

        let _ = scroll.scroll_by(Vector::ZERO, down(2.0), MAX, start);
        let mid = start + ms(50);
        let at = scroll.step(mid, MAX).expect("running");
        let _ = scroll.scroll_by(at, down(-2.0), MAX, mid);

        assert_eq!(scroll.destination(), Some(Vector::ZERO));
        assert_eq!(frames(&mut scroll, mid + ms(16)).last(), Some(&0.0));
    }

    #[test]
    fn scrolls_stop_at_the_ends() {
        let mut scroll = SmoothScroll::new();
        let now = Instant::now();
        let max = Vector::new(0.0, 50.0);

        let _ = scroll.scroll_by(Vector::ZERO, down(1.0), max, now);
        assert_eq!(scroll.destination(), Some(Vector::new(0.0, 50.0)));

        let mut at_end = SmoothScroll::new();
        let shown = at_end.scroll_by(Vector::new(0.0, 50.0), down(1.0), max, now);
        assert_eq!(shown, Vector::new(0.0, 50.0));
        assert!(!at_end.is_animating(), "nothing to animate at the end");

        let mut at_start = SmoothScroll::new();
        let _ = at_start.scroll_by(Vector::ZERO, down(-1.0), max, now);
        assert!(!at_start.is_animating(), "nothing to animate at the start");
    }

    #[test]
    fn content_that_shrinks_retargets_to_its_new_end() {
        let mut scroll = SmoothScroll::new();
        let start = Instant::now();

        let _ = scroll.scroll_by(Vector::ZERO, down(3.0), MAX, start);
        let mid = start + ms(50);
        let _ = scroll.step(mid, MAX);

        let shorter = Vector::new(0.0, 200.0);
        let mut now = mid + ms(16);
        let mut last = 0.0;

        while let Some(offset) = scroll.step(now, shorter) {
            assert!(offset.y <= 200.0 + 0.5, "past the new end: {}", offset.y);
            last = offset.y;
            now += ms(16);
        }

        assert_eq!(last, 200.0);
    }

    #[test]
    fn shift_moves_the_whole_animation() {
        let mut scroll = SmoothScroll::new();
        let now = Instant::now();

        let _ = scroll.scroll_by(Vector::ZERO, down(1.0), MAX, now);
        let before = scroll.step(now + ms(50), MAX).expect("running");

        scroll.shift(Vector::new(0.0, 30.0));

        let after = scroll.step(now + ms(50), MAX).expect("running");
        assert!((after.y - before.y - 30.0).abs() < 1e-3);
        assert_eq!(scroll.destination(), Some(Vector::new(0.0, 150.0)));
    }

    #[test]
    fn cancel_stops_where_it_is() {
        let mut scroll = SmoothScroll::new();
        let now = Instant::now();

        let _ = scroll.scroll_by(Vector::ZERO, down(1.0), MAX, now);
        scroll.cancel();

        assert!(!scroll.is_animating());
        assert_eq!(scroll.step(now + ms(50), MAX), None);
    }

    #[test]
    fn instant_threads_land_at_once() {
        SmoothScroll::set_instant(true);

        let mut scroll = SmoothScroll::new();
        let shown = scroll.scroll_by(Vector::ZERO, down(1.0), MAX, Instant::now());

        SmoothScroll::set_instant(false);

        assert_eq!(shown, down(1.0));
        assert!(!scroll.is_animating());
    }
}
