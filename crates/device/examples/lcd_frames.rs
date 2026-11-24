use std::fs;
use std::path::PathBuf;

use device::breadcrumb::{Breadcrumb, Place, Weight};
use device::control_pass::Monitoring;
use device::lcd::ballistics::{MeterFrame, OBS_RELEASE_PER_FRAME};
use device::lcd::panel::{PANEL_COLUMNS, PANEL_ROWS};
use device::lcd::screen::{
    BACKGROUND, ClockLock, DspBadge, Glass, MeterProfileName, PillFade, RegionPixels, RegionSet,
    Screen, StreamActivity, render_batch,
};
use device::phantom::PhantomPower;
use dsp::{ClippedTicks, MeterReading, PeakLevel, SampleRate};

const USAGE: &str = "usage: lcd_frames <out folder>; writes raw.ppm, eq1.ppm and eq2.ppm, the 160 x 128 panel as the sound3 screen code draws it";
const WIDTH: usize = PANEL_ROWS as usize;
const HEIGHT: usize = PANEL_COLUMNS as usize;
const SETTLE_FRAMES: usize = 24;

struct Scene {
    file: &'static str,
    dsp: DspBadge,
    rate: SampleRate,
    playing: bool,
    levels_db: [f64; 4],
}

const SCENES: [Scene; 3] = [
    Scene {
        file: "raw.ppm",
        dsp: DspBadge::Raw,
        rate: SampleRate::Hz48000,
        playing: false,
        levels_db: [-90.0; 4],
    },
    Scene {
        file: "eq1.ppm",
        dsp: DspBadge::Clarity,
        rate: SampleRate::Hz48000,
        playing: true,
        levels_db: [-38.0, -44.0, -14.5, -16.0],
    },
    Scene {
        file: "eq2.ppm",
        dsp: DspBadge::Warm,
        rate: SampleRate::Hz96000,
        playing: true,
        levels_db: [-41.0, -47.0, -7.5, -8.2],
    },
];

fn reading(db: f64) -> MeterReading {
    MeterReading {
        peak: PeakLevel::from_bits((f64::from(0x7FFF_FFFFu32) * 10f64.powf(db / 20.0)) as u32),
        clipped_ticks: ClippedTicks::from_count(0),
    }
}

fn screen(scene: &Scene) -> Screen {
    let activity = if scene.playing {
        StreamActivity::Active
    } else {
        StreamActivity::Idle
    };
    let mut screen = Screen {
        rate: scene.rate,
        clock: ClockLock::Locked,
        playback: activity,
        capture: activity,
        phantom: [PhantomPower::Off; 2],
        monitoring: [Monitoring::Off, Monitoring::On],
        meters: MeterFrame::SILENT,
        dsp: scene.dsp,
        pill: PillFade::AT_BOOT,
        profile: MeterProfileName::Obs,
        reduced_bands: None,
        breadcrumb: Breadcrumb {
            place: Place { stage: 16, step: 0 },
            code: 0,
            weight: Weight::Noted,
            latest: 16,
        },
        previous_boot: None,
    };
    let bars = scene.levels_db.map(reading);
    for _ in 0..SETTLE_FRAMES {
        screen.pill = screen.pill.next_frame(screen.pill_target(), true);
        screen.meters =
            screen
                .meters
                .next_frame_tapped(bars, [bars[2], bars[3]], 0, OBS_RELEASE_PER_FRAME);
    }
    screen
}

fn draw(screen: &Screen) -> Vec<u16> {
    let mut pixels = vec![BACKGROUND.bits(); WIDTH * HEIGHT];
    let mut glass = Glass::BLANK;
    let mut store = RegionPixels::new();
    let mut dirty = RegionSet::ALL;
    while !dirty.is_empty() {
        let batch = render_batch(dirty, screen, &mut glass, &mut store);
        for piece in batch.pieces.iter().flatten() {
            let window = piece.window;
            let height = usize::from(window.columns());
            let top = HEIGHT - usize::from(window.first_column()) - height;
            for (index, word) in store.words(*piece).iter().enumerate() {
                let x = usize::from(window.first_row()) + index / height;
                let y = top + height - 1 - index % height;
                pixels[y * WIDTH + x] = word.colour().bits();
            }
        }
        dirty = batch.leftover;
    }
    pixels
}

fn ppm(pixels: &[u16]) -> Vec<u8> {
    let mut bytes = format!("P6\n{WIDTH} {HEIGHT}\n255\n").into_bytes();
    for &pixel in pixels {
        let red = (pixel >> 11) & 0x1F;
        let green = (pixel >> 5) & 0x3F;
        let blue = pixel & 0x1F;
        bytes.extend([
            (red * 255 / 31) as u8,
            (green * 255 / 63) as u8,
            (blue * 255 / 31) as u8,
        ]);
    }
    bytes
}

fn main() {
    let out = PathBuf::from(std::env::args().nth(1).expect(USAGE));
    fs::create_dir_all(&out).expect("the output folder is writable");
    for scene in &SCENES {
        let path = out.join(scene.file);
        fs::write(&path, ppm(&draw(&screen(scene)))).expect("the frame is written");
        println!("{}", path.display());
    }
}
