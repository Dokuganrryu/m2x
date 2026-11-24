#![deny(clippy::wildcard_enum_match_arm)]

use device::console::ConsoleUsb;
use device::rate_change;

use super::handovers::USB_NOTICES;
use device::usb::BusSpeed;
use device::usb::endpoint::{
    BufferSlot, ByteCount, Command, DmaPositions, ENDPOINT_LIST_BYTES, Event, FrameStart, InEntry,
    ListEntry, OutEntry, PlaybackArrival, StreamCompletions, TransferKind,
};
use device::usb::request::{Address, HaltableEndpoint};
use device::usb::service::Service;

use super::map::{
    ANACTRL_XO32M_CTRL, ANACTRL_XO32M_CTRL_ENABLE_PLL_USB_OUT, NVIC_ICER, NVIC_ICPR, NVIC_IPR,
    NVIC_ISER, NVIC_USB1_FACTORY_PRIORITY, NVIC_USB1_IRQ, NVIC_WORD_BYTES,
    PMC_PDRUNCFG0_PDEN_LDOUSBHS, PMC_PDRUNCFG0_PDEN_USBHSPHY, PMC_PDRUNCFGCLR0,
    SYSCON_AHBCLKCTRL2_ANALOG_CTRL, SYSCON_AHBCLKCTRL2_USB1_DEV, SYSCON_AHBCLKCTRL2_USB1_HOST,
    SYSCON_AHBCLKCTRL2_USB1_PHY, SYSCON_AHBCLKCTRL2_USB1_RAM, SYSCON_AHBCLKCTRLSET_INDEX_STEP,
    SYSCON_AHBCLKCTRLSET0, USB1_RAM_ENDPOINT_LIST, USB1_RAM_FIRST_BYTE, USB1_RAM_LIST_ACTIVE,
    USB1_RAM_LIST_ADDRESS_OFFSET_MASK, USB1_RAM_LIST_ADDRESS_OFFSET_SHIFT, USB1_RAM_LIST_DISABLED,
    USB1_RAM_LIST_NBYTES_MASK, USB1_RAM_LIST_NBYTES_SHIFT, USB1_RAM_LIST_PERIODIC,
    USB1_RAM_LIST_RESTING, USB1_RAM_LIST_STALL, USB1_RAM_LIST_TOGGLE_RESET, USBHSD_DEVCMDSTAT,
    USBHSD_DEVCMDSTAT_DCON, USBHSD_DEVCMDSTAT_DCON_C, USBHSD_DEVCMDSTAT_DEV_ADDR_MASK,
    USBHSD_DEVCMDSTAT_DEV_EN, USBHSD_DEVCMDSTAT_DRES_C, USBHSD_DEVCMDSTAT_DSUS,
    USBHSD_DEVCMDSTAT_DSUS_C, USBHSD_DEVCMDSTAT_LPM_SUP, USBHSD_DEVCMDSTAT_SETUP,
    USBHSD_DEVCMDSTAT_SPEED_HIGH, USBHSD_DEVCMDSTAT_SPEED_MASK, USBHSD_DEVCMDSTAT_SPEED_SHIFT,
    USBHSD_DEVCMDSTAT_WRITE_ONE_TO_CLEAR, USBHSD_EPBUFCFG, USBHSD_EPINUSE, USBHSD_EPLISTSTART,
    USBHSD_EPSKIP, USBHSD_INTEN, USBHSD_INTSTAT, USBHSD_INTSTAT_DEV_INT, USBHSD_INTSTAT_EP0IN,
    USBHSD_INTSTAT_EP0OUT, USBHSD_INTSTAT_FRAME_INT, USBHSH_PORTMODE, USBHSH_PORTMODE_DEV_ENABLE,
    USBPHY_CTRL_CLR, USBPHY_CTRL_CLR_CLKGATE, USBPHY_CTRL_CLR_SFTRST, USBPHY_CTRL_SET,
    USBPHY_CTRL_SET_ENAUTOCLR_CLKGATE, USBPHY_CTRL_SET_ENAUTOCLR_PHY_PWD, USBPHY_PLL_SIC,
    USBPHY_PLL_SIC_FACTORY, USBPHY_PWD, USBPHY_PWD_EVERYTHING_POWERED, USBPHY_USB1_VBUS_DETECT,
    USBPHY_USB1_VBUS_DETECT_BVALID_OVERRIDE, USBPHY_USB1_VBUS_DETECT_VBUS_OVERRIDE_EN,
    USBPHY_USB1_VBUS_DETECT_VBUSVALID_OVERRIDE,
};
use super::mmio::{Register, data_memory_barrier, data_synchronization_barrier};

// WHY: UM11126 Fig 163, two list words per physical endpoint, so a list offset over eight is the physical endpoint and its INTSTAT, EPSKIP, EPINUSE and EPBUFCFG bit
const LOG2_LIST_BYTES_PER_PHYSICAL_ENDPOINT: u16 = 3;
const LIST_WORD_BYTES: u16 = 4;
const FIRST_DATA_LIST_OFFSET: u16 = 0x10;
// WHY: UM11126 Fig 163 EP3 IN buffer 0, the serial notification endpoint 0x83 of the CDC descriptors, which never sends and has no buffer
const SERIAL_NOTIFICATION_LIST_OFFSET: u16 = 0x38;
// WHY: 12,500 cycles at a 100 MHz core clock is one 125 us microframe; a poll takes at least one cycle, so this many polls outlast one microframe
#[cfg(not(feature = "sound3"))]
const SKIP_POLLS_AT_MOST: u32 = 12_500;
// WHY: The 150 MHz build, where a microframe is 18,750 cycles
#[cfg(feature = "sound3")]
const SKIP_POLLS_AT_MOST: u32 = 18_750;
const BITS_PER_PRIORITY_BYTE: usize = 8;
const PRIORITY_BYTES_PER_WORD: usize = 4;
const INTERRUPTS_PER_NVIC_WORD: usize = 32;

const USB_CLOCKS: u32 = SYSCON_AHBCLKCTRL2_USB1_HOST
    | SYSCON_AHBCLKCTRL2_USB1_DEV
    | SYSCON_AHBCLKCTRL2_USB1_RAM
    | SYSCON_AHBCLKCTRL2_USB1_PHY
    | SYSCON_AHBCLKCTRL2_ANALOG_CTRL;
const AHBCLKCTRLSET2: Register =
    SYSCON_AHBCLKCTRLSET0.bytes_after(2 * SYSCON_AHBCLKCTRLSET_INDEX_STEP);
const USB1_NVIC_WORD_BYTES: usize = NVIC_USB1_IRQ / INTERRUPTS_PER_NVIC_WORD * NVIC_WORD_BYTES;
const USB1_NVIC_BIT: u32 = 1 << (NVIC_USB1_IRQ % INTERRUPTS_PER_NVIC_WORD);
const USB1_PRIORITY_WORD_BYTES: usize = NVIC_USB1_IRQ / PRIORITY_BYTES_PER_WORD * NVIC_WORD_BYTES;
const USB1_PRIORITY_SHIFT: usize = NVIC_USB1_IRQ % PRIORITY_BYTES_PER_WORD * BITS_PER_PRIORITY_BYTE;

const CONTROL_OUT: ListEntry = ListEntry::Out(OutEntry::Control);
const CONTROL_IN: ListEntry = ListEntry::In(InEntry::Control);
const PLAYBACK_FIRST: ListEntry = ListEntry::Out(OutEntry::Playback(BufferSlot::First));
const PLAYBACK_SECOND: ListEntry = ListEntry::Out(OutEntry::Playback(BufferSlot::Second));
const CAPTURE_FIRST: ListEntry = ListEntry::In(InEntry::Capture(BufferSlot::First));
const CAPTURE_SECOND: ListEntry = ListEntry::In(InEntry::Capture(BufferSlot::Second));
const FEEDBACK: ListEntry = ListEntry::In(InEntry::Feedback);
const SERIAL_OUT: ListEntry = ListEntry::Out(OutEntry::Serial);
const SERIAL_IN: ListEntry = ListEntry::In(InEntry::Serial);

const fn physical_bit(list_offset: u16) -> u32 {
    1 << (list_offset >> LOG2_LIST_BYTES_PER_PHYSICAL_ENDPOINT)
}

const fn endpoint_bit(entry: ListEntry) -> u32 {
    physical_bit(entry.list_offset())
}

const DOUBLE_BUFFERED: u32 = endpoint_bit(PLAYBACK_FIRST) | endpoint_bit(CAPTURE_FIRST);
const DATA_ENDPOINTS: u32 = {
    let mut bits = 0;
    let mut at = 0;
    while at < ListEntry::ALL.len() {
        bits |= endpoint_bit(ListEntry::ALL[at]);
        at += 1;
    }
    bits & !(endpoint_bit(CONTROL_OUT) | endpoint_bit(CONTROL_IN))
};
const COLLECTED_AT_START_OF_FRAME: u32 =
    endpoint_bit(PLAYBACK_FIRST) | endpoint_bit(FEEDBACK) | endpoint_bit(CAPTURE_FIRST);
const INTERRUPTS_ENABLED: u32 = endpoint_bit(CONTROL_OUT)
    | endpoint_bit(CONTROL_IN)
    | endpoint_bit(SERIAL_OUT)
    | endpoint_bit(SERIAL_IN)
    | USBHSD_INTSTAT_FRAME_INT
    | USBHSD_INTSTAT_DEV_INT;
const _: () = assert!(endpoint_bit(CONTROL_OUT) == USBHSD_INTSTAT_EP0OUT);
const _: () = assert!(endpoint_bit(CONTROL_IN) == USBHSD_INTSTAT_EP0IN);
const _: () = assert!(endpoint_bit(ListEntry::Setup) == USBHSD_INTSTAT_EP0OUT);

const fn list_word(list_offset: u16) -> Register {
    USB1_RAM_ENDPOINT_LIST.bytes_after(list_offset as usize)
}

const fn buffer_field(entry: ListEntry) -> u32 {
    ((entry.region().offset as u32) >> USB1_RAM_LIST_ADDRESS_OFFSET_SHIFT)
        & USB1_RAM_LIST_ADDRESS_OFFSET_MASK
}

const fn kind_bits(entry: ListEntry) -> u32 {
    match entry.kind() {
        TransferKind::Isochronous => USB1_RAM_LIST_PERIODIC,
        TransferKind::Control | TransferKind::Bulk => 0,
    }
}

const fn resting_word(entry: ListEntry) -> u32 {
    USB1_RAM_LIST_RESTING | kind_bits(entry) | buffer_field(entry)
}

const fn armed_word(entry: ListEntry, bytes: ByteCount) -> u32 {
    USB1_RAM_LIST_ACTIVE
        | kind_bits(entry)
        | (((bytes.value() as u32) & USB1_RAM_LIST_NBYTES_MASK) << USB1_RAM_LIST_NBYTES_SHIFT)
        | buffer_field(entry)
}

const fn bytes_left(word: u32) -> ByteCount {
    ByteCount::new(((word >> USB1_RAM_LIST_NBYTES_SHIFT) & USB1_RAM_LIST_NBYTES_MASK) as u16)
}

const fn is_active(word: u32) -> bool {
    word & USB1_RAM_LIST_ACTIVE != 0
}

const fn halt_offset(endpoint: HaltableEndpoint) -> u16 {
    match endpoint {
        HaltableEndpoint::SerialNotification => SERIAL_NOTIFICATION_LIST_OFFSET,
        HaltableEndpoint::SerialReceive => SERIAL_OUT.list_offset(),
        HaltableEndpoint::SerialTransmit => SERIAL_IN.list_offset(),
    }
}

const fn halt_buffer_field(endpoint: HaltableEndpoint) -> u32 {
    match endpoint {
        HaltableEndpoint::SerialNotification => 0,
        HaltableEndpoint::SerialReceive => buffer_field(SERIAL_OUT),
        HaltableEndpoint::SerialTransmit => buffer_field(SERIAL_IN),
    }
}

const fn sibling(entry: ListEntry) -> Option<(BufferSlot, ListEntry)> {
    match entry {
        ListEntry::Out(OutEntry::Playback(slot)) => {
            Some((slot, ListEntry::Out(OutEntry::Playback(other_slot(slot)))))
        }
        ListEntry::In(InEntry::Capture(slot)) => {
            Some((slot, ListEntry::In(InEntry::Capture(other_slot(slot)))))
        }
        ListEntry::Setup
        | ListEntry::Out(OutEntry::Control | OutEntry::Serial)
        | ListEntry::In(InEntry::Control | InEntry::Feedback | InEntry::Serial) => None,
    }
}

const fn other_slot(slot: BufferSlot) -> BufferSlot {
    match slot {
        BufferSlot::First => BufferSlot::Second,
        BufferSlot::Second => BufferSlot::First,
    }
}

#[inline(always)]
fn point_controller_at(slot: BufferSlot, bit: u32) {
    let in_use = USBHSD_EPINUSE.read();
    USBHSD_EPINUSE.write(match slot {
        BufferSlot::First => in_use & !bit,
        BufferSlot::Second => in_use | bit,
    });
}

#[inline(always)]
fn slot_in_use(bit: u32) -> BufferSlot {
    match USBHSD_EPINUSE.read() & bit {
        0 => BufferSlot::First,
        _ => BufferSlot::Second,
    }
}

pub fn attach<Lock>(phy_pll_lock: impl FnOnce() -> Lock) -> Lock {
    PMC_PDRUNCFGCLR0.write(PMC_PDRUNCFG0_PDEN_USBHSPHY | PMC_PDRUNCFG0_PDEN_LDOUSBHS);
    ANACTRL_XO32M_CTRL.set(ANACTRL_XO32M_CTRL_ENABLE_PLL_USB_OUT);
    AHBCLKCTRLSET2.write(USB_CLOCKS);
    // WHY: Arm DDI 0553A.h B5.10, no rule orders the Device write that clocks USB RAM before a later Normal write to it, so B5.13.3 completes the clock write before the first list write
    data_synchronization_barrier();
    rest_every_list_word();
    // WHY: PORTMODE, the six PHY writes and the PLL lock wait come before the first USBHSD access; on hardware, writing EPLISTSTART, EPBUFCFG and EPINUSE before PORTMODE and the PHY gave no USB arrival
    USBHSH_PORTMODE.write(USBHSH_PORTMODE_DEV_ENABLE);
    USBPHY_CTRL_CLR.write(USBPHY_CTRL_CLR_SFTRST);
    USBPHY_PLL_SIC.write(USBPHY_PLL_SIC_FACTORY);
    USBPHY_CTRL_CLR.write(USBPHY_CTRL_CLR_CLKGATE);
    USBPHY_PWD.write(USBPHY_PWD_EVERYTHING_POWERED);
    USBPHY_CTRL_SET.write(USBPHY_CTRL_SET_ENAUTOCLR_CLKGATE | USBPHY_CTRL_SET_ENAUTOCLR_PHY_PWD);
    // WHY: The lock is read here, with the PHY ungated and powered, not right after the PLL_SIC write, and its answer never gates the connect below
    let lock = phy_pll_lock();
    USBPHY_USB1_VBUS_DETECT.set(
        USBPHY_USB1_VBUS_DETECT_VBUS_OVERRIDE_EN
            | USBPHY_USB1_VBUS_DETECT_BVALID_OVERRIDE
            | USBPHY_USB1_VBUS_DETECT_VBUSVALID_OVERRIDE,
    );
    // WHY: Every list write is observed before the controller registers below and the connect bit; B5.10 RRXPL lets Normal accesses be observed in any order
    data_memory_barrier();
    USBHSD_EPLISTSTART.write(USB1_RAM_FIRST_BYTE);
    USBHSD_EPBUFCFG.write(DOUBLE_BUFFERED);
    USBHSD_EPINUSE.write(0);
    NVIC_ICER
        .bytes_after(USB1_NVIC_WORD_BYTES)
        .write(USB1_NVIC_BIT);
    let priorities = NVIC_IPR.bytes_after(USB1_PRIORITY_WORD_BYTES);
    priorities.write(
        (priorities.read() & !(0xFF << USB1_PRIORITY_SHIFT))
            | (NVIC_USB1_FACTORY_PRIORITY << USB1_PRIORITY_SHIFT),
    );
    NVIC_ICPR
        .bytes_after(USB1_NVIC_WORD_BYTES)
        .write(USB1_NVIC_BIT);
    NVIC_ISER
        .bytes_after(USB1_NVIC_WORD_BYTES)
        .write(USB1_NVIC_BIT);
    USBHSD_INTEN.write(INTERRUPTS_ENABLED);
    USBHSD_DEVCMDSTAT.write(
        (USBHSD_DEVCMDSTAT.read()
            & !(USBHSD_DEVCMDSTAT_WRITE_ONE_TO_CLEAR
                | USBHSD_DEVCMDSTAT_DEV_EN
                | USBHSD_DEVCMDSTAT_LPM_SUP
                | USBHSD_DEVCMDSTAT_DCON))
            | USBHSD_DEVCMDSTAT_DEV_EN
            | USBHSD_DEVCMDSTAT_DCON,
    );
    lock
}

fn rest_every_list_word() {
    let mut offset = 0;
    while offset < ENDPOINT_LIST_BYTES {
        list_word(offset).write(USB1_RAM_LIST_DISABLED);
        offset += LIST_WORD_BYTES;
    }
    list_word(ListEntry::Setup.list_offset()).write(buffer_field(ListEntry::Setup));
    rest_control();
    disable_data_entries();
}

#[inline(always)]
fn rest_control() {
    list_word(CONTROL_OUT.list_offset()).write(resting_word(CONTROL_OUT));
    list_word(CONTROL_IN.list_offset()).write(resting_word(CONTROL_IN));
}

#[inline(always)]
fn disable_data_entries() {
    for entry in ListEntry::ALL {
        if entry.list_offset() >= FIRST_DATA_LIST_OFFSET {
            list_word(entry.list_offset()).write(USB1_RAM_LIST_DISABLED | resting_word(entry));
        }
    }
    list_word(SERIAL_NOTIFICATION_LIST_OFFSET).write(USB1_RAM_LIST_DISABLED);
}

#[unsafe(no_mangle)]
#[inline(never)]
pub fn usb_device_interrupt(
    service: &mut Service<'static>,
    clock: &rate_change::ClockReport,
    positions: &DmaPositions,
    console: &mut ConsoleUsb<'static>,
) {
    serve(positions, &mut |event| {
        let reaction = service.handle(&event, clock, console);
        for notice in reaction.notices.as_slice() {
            USB_NOTICES.publish(*notice);
        }
        let commands = reaction.commands.as_slice();
        if !commands.is_empty() {
            usb_device_commands(commands);
        }
    });
    USB_NOTICES.bus(service.function().supervised());
}

#[unsafe(no_mangle)]
#[inline(never)]
pub fn usb_device_commands(commands: &[Command]) {
    for command in commands {
        execute(*command);
    }
}

#[inline(always)]
pub fn serve(positions: &DmaPositions, report: &mut impl FnMut(Event)) {
    let raised = USBHSD_INTSTAT.read();
    let pending = raised
        & match raised & USBHSD_INTSTAT_FRAME_INT {
            0 => INTERRUPTS_ENABLED,
            _ => INTERRUPTS_ENABLED | COLLECTED_AT_START_OF_FRAME,
        };
    USBHSD_INTSTAT.write(pending);
    data_memory_barrier();
    if pending & USBHSD_INTSTAT_DEV_INT != 0 {
        device_status_changed(report);
    }
    let setup = pending & USBHSD_INTSTAT_EP0OUT != 0 && control_out_completed(report);
    if pending & USBHSD_INTSTAT_EP0IN != 0 && !setup {
        report(Event::InCompleted(InEntry::Control));
    }
    if pending & endpoint_bit(SERIAL_OUT) != 0 {
        report(Event::OutCompleted {
            entry: OutEntry::Serial,
            untransferred: bytes_left(list_word(SERIAL_OUT.list_offset()).read()),
        });
    }
    if pending & endpoint_bit(SERIAL_IN) != 0 {
        report(Event::InCompleted(InEntry::Serial));
    }
    if pending & USBHSD_INTSTAT_FRAME_INT != 0 {
        report(Event::StartOfFrame(FrameStart {
            positions: *positions,
            completed: stream_completions(pending),
        }));
    }
}

#[inline(always)]
fn stream_completions(pending: u32) -> StreamCompletions {
    let mut completed = StreamCompletions::NONE;
    if pending & endpoint_bit(PLAYBACK_FIRST) != 0 {
        let starting = slot_in_use(endpoint_bit(PLAYBACK_FIRST));
        let mut landed = completed.playback.iter_mut();
        for slot in [starting, other_slot(starting)] {
            let word = list_word(ListEntry::Out(OutEntry::Playback(slot)).list_offset()).read();
            if let (false, Some(arrival)) = (is_active(word), landed.next()) {
                *arrival = Some(PlaybackArrival {
                    slot,
                    untransferred: bytes_left(word),
                });
            }
        }
    }
    completed.feedback_sent = pending & endpoint_bit(FEEDBACK) != 0;
    if pending & endpoint_bit(CAPTURE_FIRST) != 0 {
        let starting = slot_in_use(endpoint_bit(CAPTURE_FIRST));
        let mut sent = completed.capture_sent.iter_mut();
        for slot in [starting, other_slot(starting)] {
            let word = list_word(ListEntry::In(InEntry::Capture(slot)).list_offset()).read();
            if let (false, Some(free)) = (is_active(word), sent.next()) {
                *free = Some(slot);
            }
        }
    }
    completed
}

#[inline(always)]
fn device_status_changed(report: &mut impl FnMut(Event)) {
    let status = USBHSD_DEVCMDSTAT.read();
    let changes =
        status & (USBHSD_DEVCMDSTAT_DCON_C | USBHSD_DEVCMDSTAT_DSUS_C | USBHSD_DEVCMDSTAT_DRES_C);
    USBHSD_DEVCMDSTAT.write((status & !USBHSD_DEVCMDSTAT_WRITE_ONE_TO_CLEAR) | changes);
    if changes & USBHSD_DEVCMDSTAT_DCON_C != 0 {
        report(Event::Detached);
    }
    if changes & USBHSD_DEVCMDSTAT_DSUS_C != 0 {
        report(match status & USBHSD_DEVCMDSTAT_DSUS {
            0 => Event::Resumed,
            _ => Event::Suspended,
        });
    }
    if changes & USBHSD_DEVCMDSTAT_DRES_C != 0 {
        report(Event::Attached);
        report(Event::BusReset(speed(status)));
    }
}

const fn speed(status: u32) -> BusSpeed {
    match (status >> USBHSD_DEVCMDSTAT_SPEED_SHIFT) & USBHSD_DEVCMDSTAT_SPEED_MASK {
        USBHSD_DEVCMDSTAT_SPEED_HIGH => BusSpeed::High,
        _ => BusSpeed::Full,
    }
}

#[inline(always)]
fn control_out_completed(report: &mut impl FnMut(Event)) -> bool {
    let status = USBHSD_DEVCMDSTAT.read();
    if status & USBHSD_DEVCMDSTAT_SETUP == 0 {
        report(Event::OutCompleted {
            entry: OutEntry::Control,
            untransferred: bytes_left(list_word(CONTROL_OUT.list_offset()).read()),
        });
        return false;
    }
    rest_control();
    USBHSD_INTSTAT.write(USBHSD_INTSTAT_EP0IN);
    // WHY: UM11126 rev 1.8 Table 817 SETUP and Fig 163, clearing it lets the controller answer tokens from the EP0 list words, so the release makes the resting words observed first
    USBHSD_DEVCMDSTAT
        .write_release((status & !USBHSD_DEVCMDSTAT_WRITE_ONE_TO_CLEAR) | USBHSD_DEVCMDSTAT_SETUP);
    let setup_words = USB1_RAM_ENDPOINT_LIST.bytes_after(ListEntry::Setup.region().offset as usize);
    let [a, b, c, d] = setup_words.read().to_le_bytes();
    let [e, f, g, h] = setup_words
        .bytes_after(LIST_WORD_BYTES as usize)
        .read()
        .to_le_bytes();
    report(Event::Setup([a, b, c, d, e, f, g, h]));
    true
}

#[inline(always)]
fn execute(command: Command) {
    match command {
        Command::ArmOut { entry, room } => match entry {
            OutEntry::Control => arm(CONTROL_OUT, room),
            OutEntry::Playback(BufferSlot::First) => arm(PLAYBACK_FIRST, room),
            OutEntry::Playback(BufferSlot::Second) => arm(PLAYBACK_SECOND, room),
            OutEntry::Serial => arm(SERIAL_OUT, room),
        },
        Command::ArmIn { entry, length } => match entry {
            InEntry::Control => arm(CONTROL_IN, length),
            InEntry::Feedback => arm(FEEDBACK, length),
            InEntry::Capture(BufferSlot::First) => arm(CAPTURE_FIRST, length),
            InEntry::Capture(BufferSlot::Second) => arm(CAPTURE_SECOND, length),
            InEntry::Serial => arm(SERIAL_IN, length),
        },
        Command::DisarmOut(entry) => disarm(ListEntry::Out(entry)),
        Command::DisarmIn(entry) => disarm(ListEntry::In(entry)),
        Command::StallControl => {
            list_word(CONTROL_OUT.list_offset())
                .write(USB1_RAM_LIST_STALL | resting_word(CONTROL_OUT));
            list_word(CONTROL_IN.list_offset())
                .write(USB1_RAM_LIST_STALL | resting_word(CONTROL_IN));
        }
        Command::ReleaseControlStall => rest_control(),
        Command::Halt(endpoint) => {
            let offset = halt_offset(endpoint);
            skip_until_idle(offset, None);
            list_word(offset).write(USB1_RAM_LIST_STALL | halt_buffer_field(endpoint));
        }
        Command::ReleaseHalt(endpoint) => {
            let offset = halt_offset(endpoint);
            skip_until_idle(offset, None);
            list_word(offset).write(USB1_RAM_LIST_TOGGLE_RESET | halt_buffer_field(endpoint));
        }
        Command::ApplyAddress(address) => {
            let value = match address {
                Address::Default => 0,
                Address::Assigned(assigned) => u32::from(assigned.value()),
            };
            USBHSD_DEVCMDSTAT.write(
                (USBHSD_DEVCMDSTAT.read()
                    & !(USBHSD_DEVCMDSTAT_WRITE_ONE_TO_CLEAR | USBHSD_DEVCMDSTAT_DEV_ADDR_MASK))
                    | (value & USBHSD_DEVCMDSTAT_DEV_ADDR_MASK),
            );
        }
        Command::ResetEndpoints => {
            USBHSD_EPSKIP.write(DATA_ENDPOINTS);
            disable_data_entries();
            USBHSD_EPINUSE.write(0);
            rest_control();
        }
    }
}

#[inline(always)]
fn arm(entry: ListEntry, bytes: ByteCount) {
    if let Some((slot, _)) =
        sibling(entry).filter(|(_, other)| !is_active(list_word(other.list_offset()).read()))
    {
        point_controller_at(slot, endpoint_bit(entry));
    }
    // WHY: Arm DDI 0553A.h B5.10 RVMHG orders accesses to one peripheral only, so B5.13.2 RWMRT puts the buffer writes and the EPINUSE write before the ACTIVE list word
    data_memory_barrier();
    list_word(entry.list_offset()).write(armed_word(entry, bytes));
}

#[inline(always)]
fn disarm(entry: ListEntry) {
    match entry.kind() {
        TransferKind::Control => list_word(entry.list_offset()).write(resting_word(entry)),
        TransferKind::Bulk | TransferKind::Isochronous => {
            skip_until_idle(entry.list_offset(), sibling(entry).map(|(slot, _)| slot))
        }
    }
}

#[inline(always)]
fn skip_until_idle(list_offset: u16, slot: Option<BufferSlot>) {
    if !is_active(list_word(list_offset).read()) {
        return;
    }
    let bit = physical_bit(list_offset);
    if let Some(slot) = slot {
        point_controller_at(slot, bit);
    }
    USBHSD_EPSKIP.write(bit);
    let mut polls = 0;
    while USBHSD_EPSKIP.read() & bit != 0 && polls < SKIP_POLLS_AT_MOST {
        polls += 1;
    }
}
