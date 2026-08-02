use gpui::TextRun;
use std::{collections::HashMap, ops::Range, time::Instant};

pub const VEIL_EMA_SEED_MS: f32 = 160.0;
pub const VEIL_MIN_FADE_MS: f32 = 120.0;
pub const VEIL_MAX_FADE_MS: f32 = 400.0;
pub const VEIL_CURVE_POW: f32 = 1.6;
const VEIL_GAP_CLAMP_MS: f32 = 1000.0;

#[derive(Debug, Clone)]
struct Chunk {
    range: Range<usize>,
    started: Instant,
    duration_ms: f32,
}

pub type VeilSpan = (Range<usize>, f32);

pub fn veil_opacity(progress: f32) -> f32 {
    1.0 - (1.0 - progress.clamp(0.0, 1.0)).powf(VEIL_CURVE_POW)
}

pub fn veil_duration_ms(ema_ms: f32) -> f32 {
    (ema_ms * 3.0).clamp(VEIL_MIN_FADE_MS, VEIL_MAX_FADE_MS)
}

pub fn veil_boost(active_chunks: usize) -> f32 {
    1.0 + 0.3 * active_chunks.saturating_sub(2) as f32
}

pub fn veil_ema_next(ema_ms: f32, gap_ms: f32) -> f32 {
    ema_ms * 0.7 + gap_ms.min(VEIL_GAP_CLAMP_MS) * 0.3
}

#[derive(Debug)]
struct ElementVeil {
    previous: String,
    chunks: Vec<Chunk>,
    ema_ms: f32,
    last_append: Option<Instant>,
}

impl Default for ElementVeil {
    fn default() -> Self {
        Self {
            previous: String::new(),
            chunks: Vec::new(),
            ema_ms: VEIL_EMA_SEED_MS,
            last_append: None,
        }
    }
}

fn common_prefix(left: &str, right: &str) -> usize {
    let mut prefix = left
        .as_bytes()
        .iter()
        .zip(right.as_bytes())
        .take_while(|(left, right)| left == right)
        .count();
    while prefix > 0 && !right.is_char_boundary(prefix) {
        prefix -= 1;
    }
    prefix
}

impl ElementVeil {
    fn seed(&mut self, text: &str) {
        self.previous.clear();
        self.previous.push_str(text);
    }

    fn advance(&mut self, text: &str, now: Instant) -> Vec<VeilSpan> {
        if text != self.previous {
            let prefix = common_prefix(&self.previous, text);
            self.chunks.retain_mut(|chunk| {
                chunk.range.end = chunk.range.end.min(prefix);
                chunk.range.start < chunk.range.end
            });

            if text.len() > prefix {
                if let Some(last_append) = self.last_append {
                    let gap_ms = now.saturating_duration_since(last_append).as_secs_f32() * 1000.0;
                    self.ema_ms = veil_ema_next(self.ema_ms, gap_ms);
                }
                self.last_append = Some(now);
                self.chunks.push(Chunk {
                    range: prefix..text.len(),
                    started: now,
                    duration_ms: veil_duration_ms(self.ema_ms),
                });
            }

            self.previous.clear();
            self.previous.push_str(text);
        }

        let boost = veil_boost(self.chunks.len());
        self.chunks.retain(|chunk| {
            let elapsed_ms = now.saturating_duration_since(chunk.started).as_secs_f32() * 1000.0;
            elapsed_ms * boost < chunk.duration_ms
        });

        let boost = veil_boost(self.chunks.len());
        self.chunks
            .iter()
            .map(|chunk| {
                let elapsed_ms =
                    now.saturating_duration_since(chunk.started).as_secs_f32() * 1000.0;
                let progress = (elapsed_ms * boost / chunk.duration_ms).clamp(0.0, 1.0);
                (chunk.range.clone(), veil_opacity(progress))
            })
            .collect()
    }

    fn is_fading(&self) -> bool {
        !self.chunks.is_empty()
    }
}

#[derive(Debug, Default)]
pub struct StreamingMarkdownVeil {
    elements: HashMap<usize, ElementVeil>,
    seeding: bool,
}

impl StreamingMarkdownVeil {
    pub fn seeded() -> Self {
        Self {
            elements: HashMap::default(),
            seeding: true,
        }
    }

    pub fn finish_seeding(&mut self) {
        self.seeding = false;
    }

    pub fn advance(&mut self, element: usize, text: &str, now: Instant) -> Vec<VeilSpan> {
        if self.seeding && !self.elements.contains_key(&element) {
            let mut baseline = ElementVeil::default();
            baseline.seed(text);
            self.elements.insert(element, baseline);
            return Vec::new();
        }
        self.elements.entry(element).or_default().advance(text, now)
    }

    pub fn is_fading(&self) -> bool {
        self.elements.values().any(ElementVeil::is_fading)
    }
}

pub fn apply_veil(runs: Vec<TextRun>, spans: &[VeilSpan]) -> Vec<TextRun> {
    if spans.is_empty() || spans.iter().all(|(_, opacity)| *opacity >= 1.0) {
        return runs;
    }

    let mut output = Vec::with_capacity(runs.len() + spans.len() * 2);
    let mut position = 0usize;
    for run in runs {
        let (start, end) = (position, position + run.len);
        position = end;
        let mut cuts = vec![start, end];
        for (range, _) in spans {
            if range.start > start && range.start < end {
                cuts.push(range.start);
            }
            if range.end > start && range.end < end {
                cuts.push(range.end);
            }
        }
        cuts.sort_unstable();
        cuts.dedup();

        for bounds in cuts.windows(2) {
            let (piece_start, piece_end) = (bounds[0], bounds[1]);
            let mut piece = run.clone();
            piece.len = piece_end - piece_start;
            if let Some(opacity) = spans
                .iter()
                .find(|(range, _)| range.start <= piece_start && piece_end <= range.end)
                .map(|(_, opacity)| *opacity)
                && opacity < 1.0
            {
                piece.color = piece.color.opacity(opacity);
                piece.background_color = piece.background_color.map(|color| color.opacity(opacity));
                if let Some(underline) = &mut piece.underline {
                    underline.color = underline.color.map(|color| color.opacity(opacity));
                }
                if let Some(strikethrough) = &mut piece.strikethrough {
                    strikethrough.color = strikethrough.color.map(|color| color.opacity(opacity));
                }
            }
            output.push(piece);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{font, white};
    use std::time::Duration;

    fn at(base: Instant, milliseconds: u64) -> Instant {
        base + Duration::from_millis(milliseconds)
    }

    fn run(length: usize) -> TextRun {
        TextRun {
            len: length,
            font: font("Test"),
            color: white(),
            background_color: None,
            underline: None,
            strikethrough: None,
        }
    }

    #[test]
    fn cadence_and_curve_match_omega() {
        assert_eq!(VEIL_EMA_SEED_MS, 160.0);
        assert_eq!(VEIL_MIN_FADE_MS, 120.0);
        assert_eq!(VEIL_MAX_FADE_MS, 400.0);
        assert_eq!(VEIL_CURVE_POW, 1.6);
        assert_eq!(veil_duration_ms(160.0), 400.0);
        assert_eq!(veil_duration_ms(30.0), 120.0);
        assert_eq!(veil_duration_ms(60.0), 180.0);
        assert_eq!(veil_ema_next(160.0, 100.0), 160.0 * 0.7 + 100.0 * 0.3);
        assert_eq!(veil_ema_next(160.0, 5000.0), 160.0 * 0.7 + 1000.0 * 0.3);
        assert_eq!(veil_boost(2), 1.0);
        assert!((veil_boost(3) - 1.3).abs() < f32::EPSILON);
        assert_eq!(veil_opacity(0.0), 0.0);
        assert_eq!(veil_opacity(1.0), 1.0);
        assert!(veil_opacity(0.5) > 0.5);
    }

    #[test]
    fn appended_chunks_fade_once_and_independently() {
        let start = Instant::now();
        let mut veil = ElementVeil::default();
        veil.advance("one ", start);
        let spans = veil.advance("one two ", at(start, 100));
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].0, 0..4);
        assert_eq!(spans[1].0, 4..8);
        assert!(spans[0].1 > spans[1].1);
        assert!(veil.advance("one two ", at(start, 600)).is_empty());
        assert!(veil.advance("one two ", at(start, 700)).is_empty());
    }

    #[test]
    fn seeded_veil_preserves_existing_text_and_fades_later_appends() {
        let start = Instant::now();
        let mut veil = StreamingMarkdownVeil::seeded();
        assert!(veil.advance(0, "already here", start).is_empty());
        veil.finish_seeding();
        let spans = veil.advance(0, "already here now", at(start, 100));
        assert_eq!(spans, vec![(12..16, 0.0)]);
    }

    #[test]
    fn run_splitting_changes_only_paint() {
        let output = apply_veil(vec![run(4), run(6)], &[(2..8, 0.5)]);
        assert_eq!(
            output.iter().map(|run| run.len).collect::<Vec<_>>(),
            vec![2, 2, 4, 2]
        );
        assert_eq!(output.iter().map(|run| run.len).sum::<usize>(), 10);
        assert_eq!(output[0].color.a, 1.0);
        assert_eq!(output[1].color.a, 0.5);
        assert_eq!(output[2].color.a, 0.5);
        assert_eq!(output[3].color.a, 1.0);
    }

    #[test]
    fn common_prefix_does_not_split_utf8() {
        assert_eq!(common_prefix("é", "è"), 0);
        assert_eq!(common_prefix("abé", "abè"), 2);
    }
}
