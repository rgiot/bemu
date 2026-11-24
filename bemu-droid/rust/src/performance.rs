//! Lightweight wall-clock measurements of the Android emulation loop.

use std::time::Duration;

#[derive(Default)]
pub struct Performance {
    frames: u64,
    draws: u64,
    elapsed: Duration,
    emulation: Duration,
    drawing: Duration,
    pacing: Duration,
}

impl Performance {
    pub fn record(
        &mut self,
        elapsed: Duration,
        emulation: Duration,
        drawing: Duration,
        pacing: Duration,
        attempted_draw: bool,
    ) -> Option<String> {
        self.frames += 1;
        self.draws += u64::from(attempted_draw);
        self.elapsed += elapsed;
        self.emulation += emulation;
        self.drawing += drawing;
        self.pacing += pacing;
        if self.elapsed < Duration::from_secs(2) {
            return None;
        }
        let seconds = self.elapsed.as_secs_f64();
        let millis = |duration: Duration, count: u64| {
            if count == 0 { 0.0 } else { duration.as_secs_f64() * 1000.0 / count as f64 }
        };
        let result = format!(
            "Emulation: {:.1} frames/s ({:.0}% of 50.08 Hz)\nDraw attempts: {:.1}/s\nCPU emulation: {:.1} ms/frame\nDraw submission: {:.1} ms/attempt\nPacing wait: {:.1} ms/frame",
            self.frames as f64 / seconds,
            self.frames as f64 / seconds / 50.08 * 100.0,
            self.draws as f64 / seconds,
            millis(self.emulation, self.frames),
            millis(self.drawing, self.draws),
            millis(self.pacing, self.frames),
        );
        *self = Self::default();
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_rates_and_averages_and_resets() {
        let mut stats = Performance::default();
        for n in 0..100 {
            let report = stats.record(
                Duration::from_millis(20),
                Duration::from_millis(10),
                if n % 2 == 0 { Duration::from_millis(4) } else { Duration::ZERO },
                Duration::from_millis(8),
                n % 2 == 0,
            );
            if n == 99 {
                let report = report.unwrap();
                assert!(report.contains("50.0 frames/s (100%"), "{report}");
                assert!(report.contains("Draw attempts: 25.0/s"), "{report}");
                assert!(report.contains("CPU emulation: 10.0 ms/frame"), "{report}");
                assert!(report.contains("Draw submission: 4.0 ms/attempt"), "{report}");
                assert!(report.contains("Pacing wait: 8.0 ms/frame"), "{report}");
            } else {
                assert!(report.is_none());
            }
        }
        assert!(stats.record(Duration::from_millis(20), Duration::ZERO, Duration::ZERO, Duration::ZERO, false).is_none());
    }

    #[test]
    fn reports_no_draws_without_dividing_by_zero() {
        let mut stats = Performance::default();
        let report = stats.record(
            Duration::from_secs(2),
            Duration::from_millis(40),
            Duration::ZERO,
            Duration::ZERO,
            false,
        ).unwrap();
        assert!(report.contains("Draw attempts: 0.0/s"), "{report}");
        assert!(report.contains("Draw submission: 0.0 ms/attempt"), "{report}");
    }
}
