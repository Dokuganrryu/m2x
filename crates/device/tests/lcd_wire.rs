#[cfg(not(feature = "screen2"))]
use device::breadcrumb::Breadcrumb;
#[cfg(not(feature = "screen2"))]
use device::lcd::meter::MeterFrame;
use device::lcd::panel::{INIT_STEPS, InitAction, InitStep, Opcode, RamWindow, SpiByte};
#[cfg(not(feature = "screen2"))]
use device::lcd::screen::DspBadge;
use device::lcd::screen::{BACKGROUND, Region};
#[cfg(not(feature = "screen2"))]
use device::lcd::screen::{
    ClockLock, MAX_REGION_PIXELS, RegionPixels, Screen, StreamActivity, render,
};
use device::lcd::wire::{
    Chunk, MOST_COMMAND_WORDS, MOST_TRANSFERS_PER_DESCRIPTOR, PanelWord, WINDOW_HEADER_WORDS,
    command_words, descriptor_chunks, window_header,
};
#[cfg(not(feature = "screen2"))]
use device::phantom::PhantomPower;
#[cfg(not(feature = "screen2"))]
use dsp::SampleRate;

const D_C_HIGH: u32 = 1 << 17;
const EOT: u32 = 1 << 20;
const SIXTEEN_BITS: u32 = 15 << 24;

fn is_data(word: &PanelWord) -> bool {
    word.bits() & D_C_HIGH != 0
}

fn panel_bytes(words: &[PanelWord]) -> Vec<SpiByte> {
    let mut bytes = Vec::new();
    let mut transfer_is_data = None;
    for word in words {
        let bits = word.bits();
        let data = *transfer_is_data.get_or_insert(is_data(word));
        let wrap = |byte: u8| {
            if data {
                SpiByte::Data(byte)
            } else {
                SpiByte::Command(byte)
            }
        };
        if bits & (0xF << 24) == SIXTEEN_BITS {
            bytes.push(wrap((bits >> 8) as u8));
        }
        bytes.push(wrap(bits as u8));
        if bits & EOT != 0 {
            transfer_is_data = None;
        }
    }
    bytes
}

fn assert_eot_exactly_where_the_data_select_changes_or_the_run_ends(words: &[PanelWord]) {
    for (index, word) in words.iter().enumerate() {
        let ends_a_transfer = words
            .get(index + 1)
            .is_none_or(|next| is_data(next) != is_data(word));
        assert_eq!(
            word.bits() & EOT != 0,
            ends_a_transfer,
            "word {index} of {words:08x?}"
        );
    }
}

#[test]
fn the_four_factory_fifowr_control_halves_are_reproduced_exactly() {
    assert_eq!(
        PanelWord::command(Opcode::SoftwareReset).bits() >> 16,
        0x074C
    );
    assert_eq!(
        PanelWord::command(Opcode::SoftwareReset).ending().bits() >> 16,
        0x075C
    );
    assert_eq!(PanelWord::halfword(0).bits() >> 16, 0x0F4E);
    assert_eq!(PanelWord::halfword(0).ending().bits() >> 16, 0x0F5E);
}

#[test]
fn a_parameter_byte_is_an_eight_bit_frame_with_the_data_select_high() {
    let word = PanelWord::parameter(0x3E).bits();
    assert_eq!(word & 0xFFFF, 0x3E);
    assert_ne!(word & D_C_HIGH, 0);
    assert_eq!(word & (0xF << 24), 7 << 24);
    assert_eq!(word & EOT, 0);
}

#[test]
fn every_init_command_reaches_the_panel_as_its_factory_bytes_with_eot_before_each_data_select_change()
 {
    let mut store = [PanelWord::halfword(0); MOST_COMMAND_WORDS];
    let mut sent = 0;
    for step in INIT_STEPS {
        let InitStep::Act(InitAction::Send(command)) = step else {
            continue;
        };
        let words = command_words(command, &mut store);
        assert_eq!(panel_bytes(words), command.spi_bytes().collect::<Vec<_>>());
        assert_eot_exactly_where_the_data_select_changes_or_the_run_ends(words);
        sent += 1;
    }
    assert_eq!(sent, 22);
}

#[test]
fn a_command_with_parameters_is_two_transfers_as_factory_sends_it() {
    let mut store = [PanelWord::halfword(0); MOST_COMMAND_WORDS];
    let command = INIT_STEPS
        .iter()
        .find_map(|step| match step {
            InitStep::Act(InitAction::Send(command)) if command.parameters.len() == 3 => {
                Some(*command)
            }
            _ => None,
        })
        .expect("a three-parameter init command");
    let control: Vec<u32> = command_words(command, &mut store)
        .iter()
        .map(|word| word.bits() >> 16)
        .collect();
    assert_eq!(control, [0x075C, 0x074E, 0x074E, 0x075E]);
}

#[test]
fn the_window_header_reaches_the_panel_as_the_eleven_model_bytes() {
    for window in [RamWindow::FULL]
        .into_iter()
        .chain(Region::ALL.map(Region::window))
    {
        let header = window_header(window);
        assert_eq!(header.len(), WINDOW_HEADER_WORDS);
        assert_eq!(panel_bytes(&header), window.header().to_vec());
        assert_eot_exactly_where_the_data_select_changes_or_the_run_ends(&header);
    }
}

#[cfg(not(feature = "screen2"))]
#[test]
fn a_rendered_region_is_one_transfer_whose_last_pixel_alone_carries_eot() {
    let screen = Screen {
        rate: SampleRate::Hz48000,
        clock: ClockLock::Locked,
        playback: StreamActivity::Idle,
        capture: StreamActivity::Idle,
        phantom: [PhantomPower::Off; 2],
        meters: MeterFrame::SILENT,
        dsp: DspBadge::Bypass,
        breadcrumb: Breadcrumb::START,
        previous_boot: None,
    };
    let mut store = RegionPixels::new();
    for region in Region::ALL {
        let pixels = render(region, &screen, &mut store);
        assert_eq!(pixels.len(), region.window().pixel_count());
        let (last, rest) = pixels.split_last().expect("a region has pixels");
        assert_eq!(last.bits() >> 16, 0x0F5E);
        assert!(rest.iter().all(|word| word.bits() >> 16 == 0x0F4E));
    }
    assert!(MAX_REGION_PIXELS >= Region::Meters.window().pixel_count());
    assert_eq!(PanelWord::pixel(BACKGROUND).colour(), BACKGROUND);
}

#[test]
fn a_run_is_split_into_descriptors_of_at_most_1023_transfers_that_cover_it_once() {
    for words in [1, 7, 1023, 1024, 1025, 5104, 20_479] {
        let chunks: Vec<Chunk> = descriptor_chunks(words).collect();
        let mut next = 0;
        for chunk in &chunks {
            assert_eq!(chunk.first, next);
            assert!(chunk.count >= 1 && chunk.count <= MOST_TRANSFERS_PER_DESCRIPTOR);
            next += chunk.count;
        }
        assert_eq!(next, words);
    }
    assert_eq!(MOST_TRANSFERS_PER_DESCRIPTOR, 1023);
    assert_eq!(descriptor_chunks(20_479).count(), 21);
    assert_eq!(descriptor_chunks(5104).count(), 5);
    assert_eq!(descriptor_chunks(0).count(), 0);
}

const LOW_HALF: u32 = 0xFFFF;

fn on_the_wire(colour: u16) -> u16 {
    (PanelWord::pixel(device::lcd::screen::Rgb565::from_bits(colour)).bits() & LOW_HALF) as u16
}

// WHY: The factory clip word is 0x0F4E001F and a clip box shows red on the panel; with MADCTL 0x00 the panel takes the low five bits as red
#[test]
fn a_red_pixel_goes_out_as_the_factory_clip_word() {
    assert_eq!(on_the_wire(0xF800), 0x001F);
    assert_eq!(on_the_wire(0x001F), 0xF800);
}

// WHY: The factory meter strip runs 0x0606 at its foot to 0x01BE at its top, green to red only if the panel reads the fields as blue, green, red
#[test]
fn the_factory_meter_strip_reads_green_at_the_foot_and_red_at_the_top() {
    let seen = |word: u16| PanelWord::halfword(word).colour().bits();
    let fields = |bits: u16| (bits >> 11, (bits >> 5) & 0x3F, bits & 0x1F);
    let (foot_red, foot_green, foot_blue) = fields(seen(0x0606));
    assert!(foot_green > 2 * foot_red && foot_green > 2 * foot_blue);
    let (top_red, top_green, top_blue) = fields(seen(0x01BE));
    assert!(top_red == 30 && top_blue == 0 && top_green < 2 * top_red);
}

#[test]
fn colours_with_equal_red_and_blue_go_out_unchanged() {
    for colour in [
        BACKGROUND.bits(),
        0x0000,
        0xFFFF,
        0x07E0,
        0x2104,
        0x4208,
        0x1082,
    ] {
        assert_eq!(on_the_wire(colour), colour);
    }
}

#[test]
fn a_pixel_read_back_from_the_wire_is_the_colour_that_was_drawn() {
    for colour in [0xF800, 0xFD20, 0x8000, 0x001F, 0x1234, 0xFFFF] {
        let word = PanelWord::pixel(device::lcd::screen::Rgb565::from_bits(colour));
        assert_eq!(word.colour().bits(), colour);
    }
}
