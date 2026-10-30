use std::time::Duration;

/// Mono PCM audio, normalized to `[-1.0, 1.0]`.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioClip {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

impl AudioClip {
    pub fn new(samples: Vec<f32>, sample_rate: u32) -> Self {
        Self {
            samples,
            sample_rate,
        }
    }

    pub fn duration(&self) -> Duration {
        if self.sample_rate == 0 {
            return Duration::ZERO;
        }
        Duration::from_secs_f64(self.samples.len() as f64 / f64::from(self.sample_rate))
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_from_sample_count() {
        let clip = AudioClip::new(vec![0.0; 24_000], 16_000);
        assert_eq!(clip.duration(), Duration::from_millis(1500));
    }

    #[test]
    fn zero_sample_rate_is_zero_duration() {
        let clip = AudioClip::new(vec![0.0; 10], 0);
        assert_eq!(clip.duration(), Duration::ZERO);
    }
}
