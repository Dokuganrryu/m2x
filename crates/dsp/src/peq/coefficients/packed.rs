use core::sync::atomic::{AtomicU32, Ordering};

use super::{BandCoefficients, CoefficientFormat, CoefficientSet, Generation, QuantisedBiquad};
use crate::peq::{MOST_BANDS, PEQ_BANDS};
use crate::rate::SampleRate;

const WORDS_PER_BIQUAD: usize = 5;
// WHY: Word 0 holds the rate and the enabled mask, word 1 a 2-bit tag per band, so up to 16 bands fit two words
const HEADER_WORDS: usize = 2;
const SLOT_WORDS: usize = HEADER_WORDS + PEQ_BANDS * WORDS_PER_BIQUAD;
const RATE_FIELD_MASK: u32 = 0b111;
const BAND_TAG_BITS: u32 = 2;
const BAND_TAG_MASK: u32 = 0b11;
const ENABLED_MASK_SHIFT: u32 = 3;
const ENABLED_MASK_FIELD: u32 = (1 << PEQ_BANDS) - 1;

const _: () = assert!(ENABLED_MASK_SHIFT + MOST_BANDS as u32 <= u32::BITS);
const _: () = assert!(BAND_TAG_BITS * MOST_BANDS as u32 <= u32::BITS);

#[derive(Clone, Copy)]
enum BandTag {
    Skip,
    Filter(CoefficientFormat),
}

impl BandTag {
    const fn of(band: &BandCoefficients) -> Self {
        match band {
            BandCoefficients::Skip => Self::Skip,
            BandCoefficients::Filter(biquad) => Self::Filter(biquad.format),
        }
    }

    const fn bits(self) -> u32 {
        match self {
            Self::Skip => 0,
            Self::Filter(CoefficientFormat::Q2_30) => 1,
            Self::Filter(CoefficientFormat::Q3_29) => 2,
            Self::Filter(CoefficientFormat::Q4_28) => 3,
        }
    }

    const fn from_bits(bits: u32) -> Self {
        match bits & BAND_TAG_MASK {
            0 => Self::Skip,
            1 => Self::Filter(CoefficientFormat::Q2_30),
            2 => Self::Filter(CoefficientFormat::Q3_29),
            _ => Self::Filter(CoefficientFormat::Q4_28),
        }
    }
}

const fn rate_bits(rate: SampleRate) -> u32 {
    match rate {
        SampleRate::Hz44100 => 0,
        SampleRate::Hz48000 => 1,
        SampleRate::Hz88200 => 2,
        SampleRate::Hz96000 => 3,
        SampleRate::Hz176400 => 4,
        SampleRate::Hz192000 => 5,
    }
}

const fn rate_from_bits(bits: u32) -> SampleRate {
    match bits & RATE_FIELD_MASK {
        0 => SampleRate::Hz44100,
        1 => SampleRate::Hz48000,
        2 => SampleRate::Hz88200,
        3 => SampleRate::Hz96000,
        4 => SampleRate::Hz176400,
        _ => SampleRate::Hz192000,
    }
}

impl Generation {
    pub(crate) const fn from_published(word: u32) -> Self {
        Self(word)
    }
}

pub(crate) struct PackedCoefficientSet {
    words: [AtomicU32; SLOT_WORDS],
}

impl PackedCoefficientSet {
    pub(crate) const fn flat_at(rate: SampleRate) -> Self {
        let mut words = [const { AtomicU32::new(0) }; SLOT_WORDS];
        words[0] = AtomicU32::new(rate_bits(rate));
        Self { words }
    }

    pub(crate) fn store(&self, set: &CoefficientSet) {
        let [header, tags, coefficients @ ..] = &self.words;
        let header_bits = rate_bits(set.rate) | u32::from(set.enabled_mask) << ENABLED_MASK_SHIFT;
        let mut tag_bits = 0;
        let lanes = set
            .bands
            .iter()
            .zip(coefficients.chunks_exact(WORDS_PER_BIQUAD))
            .enumerate();
        for (index, (band, words)) in lanes {
            tag_bits |= BandTag::of(band).bits() << (BAND_TAG_BITS * index as u32);
            let biquad_words = match band {
                BandCoefficients::Skip => [0; WORDS_PER_BIQUAD],
                BandCoefficients::Filter(biquad) => {
                    let [b0, b1, b2] = biquad.feedforward;
                    let [f1, f2] = biquad.feedback;
                    [b0, b1, b2, f1, f2]
                }
            };
            for (cell, word) in words.iter().zip(biquad_words) {
                cell.store(word as u32, Ordering::Relaxed);
            }
        }
        tags.store(tag_bits, Ordering::Relaxed);
        header.store(header_bits, Ordering::Relaxed);
    }

    pub(crate) fn load(&self, generation: Generation) -> CoefficientSet {
        let [header, tags, coefficients @ ..] = &self.words;
        let header_bits = header.load(Ordering::Relaxed);
        let tag_bits = tags.load(Ordering::Relaxed);
        let mut bands = [BandCoefficients::Skip; PEQ_BANDS];
        let lanes = bands
            .iter_mut()
            .zip(coefficients.chunks_exact(WORDS_PER_BIQUAD))
            .enumerate();
        for (index, (band, words)) in lanes {
            let tag = tag_bits >> (BAND_TAG_BITS * index as u32);
            *band = match BandTag::from_bits(tag) {
                BandTag::Skip => BandCoefficients::Skip,
                BandTag::Filter(format) => {
                    let mut biquad_words = [0i32; WORDS_PER_BIQUAD];
                    for (word, cell) in biquad_words.iter_mut().zip(words) {
                        *word = cell.load(Ordering::Relaxed) as i32;
                    }
                    let [b0, b1, b2, f1, f2] = biquad_words;
                    BandCoefficients::Filter(QuantisedBiquad {
                        feedforward: [b0, b1, b2],
                        feedback: [f1, f2],
                        format,
                    })
                }
            };
        }
        CoefficientSet {
            rate: rate_from_bits(header_bits),
            bands,
            enabled_mask: (header_bits >> ENABLED_MASK_SHIFT & ENABLED_MASK_FIELD) as u16,
            generation,
        }
    }
}
