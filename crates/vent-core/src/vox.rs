/// Voice-activated transmit gate — pure DSP, no platform APIs.
///
/// Port of VentMac/VentiPhone `VoxGate`. Feed 16-bit LE PCM chunks; time is
/// advanced from chunk duration so the gate is deterministic.

#[derive(Clone, Debug, PartialEq)]
pub struct VoxConfig {
    pub open_threshold_dbfs: f32,
    pub close_threshold_dbfs: f32,
    pub hangover_ms: f64,
    pub min_on_ms: f64,
    pub pre_roll_ms: f64,
}

impl Default for VoxConfig {
    fn default() -> Self {
        Self {
            open_threshold_dbfs: -40.0,
            close_threshold_dbfs: -50.0,
            hangover_ms: 300.0,
            min_on_ms: 150.0,
            pre_roll_ms: 150.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoxChunk {
    pub pcm: Vec<u8>,
    pub rate: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VoxAction {
    Idle,
    Open(Vec<VoxChunk>),
    Transmit(VoxChunk),
    Close,
}

pub struct VoxGate {
    pub config: VoxConfig,
    muted: bool,
    pending_close_from_mute: bool,
    is_open: bool,
    last_level_dbfs: f32,
    clock_ms: f64,
    opened_at_ms: f64,
    last_above_close_ms: f64,
    preroll: Vec<(VoxChunk, f64)>,
}

impl Default for VoxGate {
    fn default() -> Self {
        Self::new(VoxConfig::default())
    }
}

impl VoxGate {
    pub fn new(config: VoxConfig) -> Self {
        Self {
            config,
            muted: false,
            pending_close_from_mute: false,
            is_open: false,
            last_level_dbfs: -120.0,
            clock_ms: 0.0,
            opened_at_ms: 0.0,
            last_above_close_ms: 0.0,
            preroll: Vec::new(),
        }
    }

    pub fn is_open(&self) -> bool {
        self.is_open
    }
    pub fn last_level_dbfs(&self) -> f32 {
        self.last_level_dbfs
    }
    pub fn muted(&self) -> bool {
        self.muted
    }
    pub fn set_muted(&mut self, muted: bool) {
        if muted && self.is_open {
            self.pending_close_from_mute = true;
        }
        self.muted = muted;
    }

    pub fn reset(&mut self) {
        self.is_open = false;
        self.clock_ms = 0.0;
        self.opened_at_ms = 0.0;
        self.last_above_close_ms = 0.0;
        self.preroll.clear();
        self.pending_close_from_mute = false;
        self.last_level_dbfs = -120.0;
    }

    pub fn process(&mut self, pcm: &[u8], rate: u32) -> VoxAction {
        let samples = pcm.len() / 2;
        let dur_ms = if rate > 0 {
            samples as f64 / rate as f64 * 1000.0
        } else {
            0.0
        };
        self.clock_ms += dur_ms;
        self.last_level_dbfs = level_dbfs(pcm);
        let chunk = VoxChunk {
            pcm: pcm.to_vec(),
            rate,
        };

        if self.muted {
            self.preroll.clear();
            if self.is_open || self.pending_close_from_mute {
                self.is_open = false;
                self.pending_close_from_mute = false;
                return VoxAction::Close;
            }
            return VoxAction::Idle;
        }

        if !self.is_open {
            self.preroll.push((chunk.clone(), self.clock_ms));
            while self
                .preroll
                .first()
                .is_some_and(|(_, end)| self.clock_ms - *end > self.config.pre_roll_ms)
            {
                self.preroll.remove(0);
            }
            if self.last_level_dbfs < self.config.open_threshold_dbfs {
                return VoxAction::Idle;
            }
            self.is_open = true;
            self.opened_at_ms = self.clock_ms;
            self.last_above_close_ms = self.clock_ms;
            let flush: Vec<VoxChunk> = self.preroll.drain(..).map(|(c, _)| c).collect();
            return VoxAction::Open(flush);
        }

        if self.last_level_dbfs >= self.config.close_threshold_dbfs {
            self.last_above_close_ms = self.clock_ms;
        }
        let held_for = self.clock_ms - self.opened_at_ms;
        let since_above = self.clock_ms - self.last_above_close_ms;
        if since_above > self.config.hangover_ms && held_for > self.config.min_on_ms {
            self.is_open = false;
            return VoxAction::Close;
        }
        VoxAction::Transmit(chunk)
    }
}

pub fn level_dbfs(pcm: &[u8]) -> f32 {
    let n = pcm.len() / 2;
    if n == 0 {
        return -120.0;
    }
    let mut sum_sq = 0.0f64;
    for i in 0..n {
        let v = i16::from_le_bytes([pcm[i * 2], pcm[i * 2 + 1]]) as f64 / 32768.0;
        sum_sq += v * v;
    }
    let rms = (sum_sq / n as f64).sqrt();
    if rms > 0.0 {
        (20.0 * rms.log10()) as f32
    } else {
        -120.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm(dbfs: f32, ms: f64, rate: u32) -> Vec<u8> {
        let n = (rate as f64 * ms / 1000.0) as usize;
        let amp = if dbfs <= -120.0 {
            0.0
        } else {
            10f64.powf(dbfs as f64 / 20.0)
        };
        let v = (amp * 32767.0).clamp(-32767.0, 32767.0) as i16;
        let mut d = Vec::with_capacity(n * 2);
        for _ in 0..n {
            d.extend_from_slice(&v.to_le_bytes());
        }
        d
    }

    #[test]
    fn level_meter() {
        let a = level_dbfs(&pcm(-20.0, 40.0, 48_000));
        assert!((a - -20.0).abs() < 0.5, "{a}");
        let b = level_dbfs(&pcm(-120.0, 40.0, 48_000));
        assert!((b - -120.0).abs() < 0.1, "{b}");
    }

    #[test]
    fn stays_closed_below_threshold() {
        let mut g = VoxGate::default();
        assert_eq!(
            g.process(&pcm(-60.0, 40.0, 48_000), 48_000),
            VoxAction::Idle
        );
        assert!(!g.is_open());
    }

    #[test]
    fn opens_above_threshold() {
        let mut g = VoxGate::default();
        match g.process(&pcm(-20.0, 40.0, 48_000), 48_000) {
            VoxAction::Open(flush) => {
                assert!(g.is_open());
                assert!(!flush.is_empty());
            }
            other => panic!("expected open, got {other:?}"),
        }
    }

    #[test]
    fn preroll_flushed() {
        let mut g = VoxGate::default();
        let _ = g.process(&pcm(-45.0, 40.0, 48_000), 48_000);
        let _ = g.process(&pcm(-45.0, 40.0, 48_000), 48_000);
        match g.process(&pcm(-15.0, 40.0, 48_000), 48_000) {
            VoxAction::Open(flush) => assert!(flush.len() >= 2),
            other => panic!("expected open, got {other:?}"),
        }
    }

    #[test]
    fn closes_after_hangover() {
        let mut g = VoxGate::default();
        let _ = g.process(&pcm(-15.0, 40.0, 48_000), 48_000);
        let mut closed = false;
        for _ in 0..20 {
            if matches!(
                g.process(&pcm(-90.0, 40.0, 48_000), 48_000),
                VoxAction::Close
            ) {
                closed = true;
                break;
            }
        }
        assert!(closed);
        assert!(!g.is_open());
    }

    #[test]
    fn mute_closes_immediately() {
        let mut g = VoxGate::default();
        let _ = g.process(&pcm(-15.0, 40.0, 48_000), 48_000);
        assert!(g.is_open());
        g.set_muted(true);
        assert_eq!(
            g.process(&pcm(-15.0, 40.0, 48_000), 48_000),
            VoxAction::Close
        );
        assert!(!g.is_open());
    }
}
