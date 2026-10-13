//! How fast a run is going, and how long it has left.
//!
//! **Messages read, not files fetched.** The progress events count messages as
//! the read loop passes them, and the media pass for a chat runs after its
//! read — so "time left" is the time left *reading this chat*, and says so. A
//! figure for the whole queue would need every later chat's size, which is
//! only known once each one starts.
//!
//! The speed is measured over the last [`WINDOW`] rather than since the start,
//! because a run's speed is not one number: a two-minute rate-limit wait in
//! the first chat would otherwise drag the figure down for the rest of the
//! run, and a short burst at the start would inflate it.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How far back the speed looks.
const WINDOW: Duration = Duration::from_secs(30);
/// Below this much history the speed is noise, and none is shown.
const MIN_SPAN: Duration = Duration::from_secs(3);

/// One run's clock and its recent progress.
#[derive(Debug, Default)]
pub struct Pace {
    started: Option<Instant>,
    /// `(when, messages read so far in this run)`, oldest first.
    samples: VecDeque<(Instant, usize)>,
}

impl Pace {
    /// A run began. Forgets the last one.
    pub fn start(&mut self, now: Instant) {
        self.started = Some(now);
        self.samples.clear();
    }

    /// The run ended. Returns how long it took.
    pub fn stop(&mut self, now: Instant) -> Option<Duration> {
        self.samples.clear();
        self.started.take().map(|s| now.saturating_duration_since(s))
    }

    pub fn elapsed(&self, now: Instant) -> Option<Duration> {
        self.started.map(|s| now.saturating_duration_since(s))
    }

    /// `read` is the run's running total of messages read.
    pub fn sample(&mut self, now: Instant, read: usize) {
        if self.started.is_none() {
            return;
        }
        // A total that went down is a resumed chat starting again from the
        // top; what came before it no longer measures anything.
        if self.samples.back().is_some_and(|&(_, last)| read < last) {
            self.samples.clear();
        }
        self.samples.push_back((now, read));
        // Keep one sample older than the window, so the span reaching back
        // to it is never shorter than the window once the run is that old.
        while self.samples.len() > 2
            && now.saturating_duration_since(self.samples[1].0) > WINDOW
        {
            self.samples.pop_front();
        }
    }

    /// Messages a second over the recent window, or `None` while there is too
    /// little to go on.
    pub fn rate(&self) -> Option<f64> {
        let (&(t0, n0), &(t1, n1)) = (self.samples.front()?, self.samples.back()?);
        let span = t1.saturating_duration_since(t0);
        if span < MIN_SPAN || n1 <= n0 {
            return None;
        }
        Some((n1 - n0) as f64 / span.as_secs_f64())
    }

    /// How long reading `remaining` more messages should take at the current
    /// rate.
    pub fn left(&self, remaining: i64) -> Option<Duration> {
        let rate = self.rate()?;
        (remaining > 0).then(|| Duration::from_secs_f64(remaining as f64 / rate))
    }
}

/// `4:07`, or `1:02:07` past an hour: a clock, because the figure changes
/// every second and a clock is the shape people read that in.
pub fn clock(d: Duration) -> String {
    let s = d.as_secs();
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// `about 3 min left`. Rounded, because an estimate printed to the second
/// claims a precision it does not have.
pub fn left_text(d: Duration) -> String {
    let mins = (d.as_secs() + 30) / 60;
    match mins {
        0 => "under a minute left".into(),
        1..=59 => format!("about {mins} min left"),
        _ => format!("about {} h {} min left", mins / 60, mins % 60),
    }
}

/// `410 messages/s`, or `2.5 messages/s` while it is slow.
pub fn rate_text(rate: f64) -> String {
    if rate >= 10.0 {
        format!("{rate:.0} messages/s")
    } else {
        format!("{rate:.1} messages/s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn no_speed_until_there_is_something_to_measure() {
        let t = Instant::now();
        let mut p = Pace::default();
        p.start(t);
        assert_eq!(p.rate(), None);
        p.sample(t, 0);
        p.sample(t + secs(1), 100);
        assert_eq!(p.rate(), None, "one second is noise");
        p.sample(t + secs(4), 400);
        assert_eq!(p.rate(), Some(100.0));
    }

    #[test]
    fn the_speed_forgets_an_old_wait() {
        // A two-minute rate limit at the start must not hold the figure down
        // for the rest of the run.
        let t = Instant::now();
        let mut p = Pace::default();
        p.start(t);
        p.sample(t, 0);
        p.sample(t + secs(120), 10);
        for i in 1..=40 {
            p.sample(t + secs(120 + i), 10 + 100 * i as usize);
        }
        let rate = p.rate().unwrap();
        assert!((95.0..=105.0).contains(&rate), "{rate}");
    }

    #[test]
    fn a_restarted_chat_does_not_read_as_negative_progress() {
        let t = Instant::now();
        let mut p = Pace::default();
        p.start(t);
        p.sample(t, 5_000);
        p.sample(t + secs(5), 0);
        p.sample(t + secs(10), 500);
        assert_eq!(p.rate(), Some(100.0));
    }

    #[test]
    fn nothing_is_sampled_outside_a_run() {
        let t = Instant::now();
        let mut p = Pace::default();
        p.sample(t, 10);
        assert!(p.samples.is_empty());
        p.start(t);
        assert_eq!(p.stop(t + secs(65)), Some(secs(65)));
        assert_eq!(p.elapsed(t + secs(70)), None);
    }

    #[test]
    fn time_left_is_remaining_over_rate() {
        let t = Instant::now();
        let mut p = Pace::default();
        p.start(t);
        p.sample(t, 0);
        p.sample(t + secs(10), 1_000);
        assert_eq!(p.left(6_000), Some(secs(60)));
        assert_eq!(p.left(0), None);
    }

    #[test]
    fn the_words_are_rounded_like_an_estimate() {
        assert_eq!(clock(secs(247)), "4:07");
        assert_eq!(clock(secs(3727)), "1:02:07");
        assert_eq!(left_text(secs(20)), "under a minute left");
        assert_eq!(left_text(secs(170)), "about 3 min left");
        assert_eq!(left_text(secs(7_800)), "about 2 h 10 min left");
        assert_eq!(rate_text(410.4), "410 messages/s");
        assert_eq!(rate_text(2.46), "2.5 messages/s");
    }
}
