//! Context Mixing model (PAQ-style) — v0 simplified.
//!
//! We mix two specialized predictors (order-0 and order-1) with normalized
//! weights in the log domain. The mixer is a tiny LMS update loop.
//!
//! In v1 each context model becomes a small MLP/CNN and the mixer becomes a
//! 2-layer sigmoid network. The data flow is identical.

const N_CTX: usize = 2;

#[derive(Debug)]
pub struct CmModel {
    pub o0: [u32; 256],
    pub o1: [[u32; 256]; 256],
    pub last: u8,
    pub total_obs: u32,
    pub w: [f32; N_CTX],
}

impl Default for CmModel {
    fn default() -> Self {
        Self {
            o0: [1u32; 256],
            o1: [[1u32; 256]; 256],
            last: 0,
            total_obs: 0,
            w: [1.0, 1.0],
        }
    }
}

impl CmModel {
    pub fn observe(&mut self, b: u8) {
        self.o0[b as usize] += 1;
        self.o1[self.last as usize][b as usize] += 1;
        self.last = b;
        self.total_obs += 1;
    }

    /// Returns a non-normalized probability distribution over 256 bytes.
    /// Caller scales to a FreqTable.
    pub fn distribution(&self) -> [u32; 256] {
        let total0: f32 = self.o0.iter().map(|&c| c as f32).sum::<f32>().max(1.0);
        let total1: f32 = self.o1[self.last as usize]
            .iter()
            .map(|&c| c as f32)
            .sum::<f32>()
            .max(1.0);

        let sum_w = self.w[0] + self.w[1];
        let w0 = self.w[0] / sum_w;
        let w1 = self.w[1] / sum_w;

        // Compute raw probabilities in float, then scale by a large constant
        // before casting to u32 — otherwise values < 1.0 get truncated to 0.
        const SCALE: f32 = 1_000_000.0;
        let mut out = [0u32; 256];
        for i in 0..256usize {
            let p0 = self.o0[i] as f32 / total0;
            let p1 = self.o1[self.last as usize][i] as f32 / total1;
            let lp0 = if p0 > 0.0 { p0.ln() } else { -50.0 };
            let lp1 = if p1 > 0.0 { p1.ln() } else { -50.0 };
            let log_mix = w0 * lp0 + w1 * lp1;
            out[i] = (log_mix.exp() * SCALE).max(1.0) as u32;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distribution_favors_seen_bytes() {
        let mut m = CmModel::default();
        for &b in b"hello world hello" {
            m.observe(b);
        }
        let d = m.distribution();
        assert!(
            d[b'l' as usize] > d[b'q' as usize],
            "l should outrank q in 'hello world hello'"
        );
        assert!(
            d[b'o' as usize] > d[b'z' as usize],
            "o should outrank z"
        );
    }
}