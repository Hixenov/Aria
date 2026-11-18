//! How much delay the resampler adds, measured with an impulse.

use aria_audio::Resampler;

/// Feeds a one-channel impulse at frame 0 in blocks of `block` frames, as a capture device
/// delivers them, and returns the delay the resampler adds, in milliseconds: from the end of
/// the block that held the impulse to the end of the block after which it came out.
fn added_delay_ms(mut resampler: Resampler, from: u32, block: usize) -> f64 {
    let mut out = Vec::new();
    // How much output there was after each block.
    let mut lengths = Vec::new();
    let blocks = from as usize / 2 / block;
    for index in 0..blocks {
        let mut input = vec![0.0f32; block];
        if index == 0 {
            input[0] = 1.0;
        }
        if let Err(error) = resampler.process(&input, &mut out) {
            panic!("the resampler failed: {error}");
        }
        lengths.push(out.len());
    }
    let mut peak = 0;
    for (at, sample) in out.iter().enumerate() {
        if sample.abs() > out[peak].abs() {
            peak = at;
        }
    }
    // The first block after which the peak had come out.
    let Some(after) = lengths.iter().position(|length| *length > peak) else {
        panic!("the impulse never came out");
    };
    after as f64 * block as f64 * 1_000.0 / f64::from(from)
}

const PAIRS: [(u32, u32); 5] = [
    (48_000, 44_100),
    (44_100, 48_000),
    (16_000, 48_000),
    (16_000, 44_100),
    (96_000, 48_000),
];

#[test]
fn the_music_resampler_works_in_big_chunks() {
    // For the music, which is decoded ahead, the delay does not matter; big chunks cost less.
    for (from, to) in PAIRS {
        let resampler = Resampler::new(from, to, 1).unwrap();
        let ms = added_delay_ms(resampler, from, from as usize / 100);
        let fine = added_delay_ms(
            Resampler::new(from, to, 1).unwrap(),
            from,
            from as usize / 1_000,
        );
        println!(
            "music, {from} -> {to} Hz: {ms:.1} ms in 10 ms blocks, {fine:.1} ms in 1 ms blocks"
        );
    }
}

#[test]
fn the_live_resampler_adds_at_most_ten_milliseconds() {
    // For the microphone: what a device period of 10 ms brings in comes out by the next one, at
    // the latest, whatever the rates.
    for (from, to) in PAIRS {
        let resampler = Resampler::live(from, to, 1).unwrap();
        let ms = added_delay_ms(resampler, from, from as usize / 100);
        assert!(ms <= 10.0, "{from} -> {to} Hz adds {ms:.1} ms");
        // In blocks of 1 ms, nearer what it holds back itself: its chunk and its filter.
        let fine = added_delay_ms(
            Resampler::live(from, to, 1).unwrap(),
            from,
            from as usize / 1_000,
        );
        println!(
            "live, {from} -> {to} Hz: {ms:.1} ms in 10 ms blocks, {fine:.1} ms in 1 ms blocks"
        );
        assert!(fine <= 10.0, "{from} -> {to} Hz adds {fine:.1} ms");
    }
}
