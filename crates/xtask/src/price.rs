mod command;
mod cycles;
mod listing;
mod proxy_table;
mod share;

pub use command::command;
pub use cycles::{InstructionCount, ProxyCycles, TenthsOfAPercent, WHOLE_TICK};
pub use listing::{ListingError, ListingErrorKind};
pub use share::{
    Basis, Block, DSP_SHARE_CAP, Share, TICK_AND_USB_CAP, TickMode, USB_INTERRUPT_CAPS, UsbPath,
};

use dsp::FramesPerTick;
use listing::{BlockPath, Instruction};
use proxy_table::{Flow, classify, proxy_cycles};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockPrice {
    pub block: Block,
    pub frames: FramesPerTick,
    pub instructions: InstructionCount,
    pub cycles: ProxyCycles,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    WithinShare(ProxyCycles),
    OverShare(ProxyCycles),
    OffAtThisFrameCount,
}

pub fn price_listing(text: &str) -> Result<Vec<BlockPrice>, ListingError> {
    listing::block_paths(text)?.iter().map(price_path).collect()
}

fn price_path(path: &BlockPath<'_>) -> Result<BlockPrice, ListingError> {
    let next_addresses = path
        .instructions
        .iter()
        .skip(1)
        .map(|next| Some(next.address))
        .chain([None]);
    let cycles = path
        .instructions
        .iter()
        .zip(next_addresses)
        .map(|(instruction, next)| price_instruction(instruction, next))
        .sum::<Result<ProxyCycles, ListingError>>()?;
    Ok(BlockPrice {
        block: path.block,
        frames: path.frames,
        instructions: InstructionCount::new(path.instructions.len() as u32),
        cycles,
    })
}

fn price_instruction(
    instruction: &Instruction<'_>,
    next_address: Option<u32>,
) -> Result<ProxyCycles, ListingError> {
    let flow = match next_address {
        Some(next) if next == instruction.address + instruction.size => Flow::FallsThrough,
        _ => Flow::Jumps,
    };
    proxy_cycles(classify(instruction.mnemonic), instruction.operands, flow).ok_or_else(|| {
        ListingError {
            line: instruction.line,
            kind: ListingErrorKind::NoProxyPrice(instruction.mnemonic.to_string()),
        }
    })
}

pub(crate) const CONDITION_FAILED_INSTRUCTION: ProxyCycles = ProxyCycles::new(1);

pub(crate) struct DecodedInstruction {
    pub address: u32,
    pub size: u32,
    pub mnemonic: String,
    pub operands: String,
    pub falls_through: Option<ProxyCycles>,
    pub jumps: Option<ProxyCycles>,
}

pub(crate) fn decode_instruction_listing_line(text: &str) -> Option<DecodedInstruction> {
    let instruction = listing::read_instruction(0, text.trim()).ok()?;
    let class = classify(instruction.mnemonic);
    Some(DecodedInstruction {
        address: instruction.address,
        size: instruction.size,
        mnemonic: instruction.mnemonic.to_string(),
        operands: instruction.operands.to_string(),
        falls_through: proxy_cycles(class, instruction.operands, Flow::FallsThrough),
        jumps: proxy_cycles(class, instruction.operands, Flow::Jumps),
    })
}

impl BlockPrice {
    pub fn verdict(&self) -> Verdict {
        match self.block.share(self.frames) {
            Share::Cap(cap) if self.cycles <= cap => Verdict::WithinShare(cap),
            Share::Cap(cap) => Verdict::OverShare(cap),
            Share::OffAtThisFrameCount => Verdict::OffAtThisFrameCount,
        }
    }

    pub fn passes(&self) -> bool {
        matches!(self.verdict(), Verdict::WithinShare(_))
    }
}

impl fmt::Display for BlockPrice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let margin = self.cycles.with_unpublished_m33_margin();
        let gate = if self.passes() { "PASS" } else { "FAIL" };
        write!(
            f,
            "{gate} {} at {} frames: {} instructions, {} proxy cycles, {} of the {}-cycle tick",
            self.block.id(),
            self.frames.count(),
            self.instructions,
            self.cycles,
            self.cycles.share_of_whole_tick(),
            WHOLE_TICK,
        )?;
        match self.verdict() {
            Verdict::WithinShare(cap) => write!(
                f,
                ", within its D3 share of {} ({})",
                cap,
                cap.share_of_whole_tick()
            )?,
            Verdict::OverShare(cap) => write!(
                f,
                ", over its D3 share of {} ({}) by {}",
                cap,
                cap.share_of_whole_tick(),
                ProxyCycles::new(self.cycles.get() - cap.get())
            )?,
            Verdict::OffAtThisFrameCount => {
                f.write_str(", but D4 turns this block off at this frame count")?
            }
        }
        write!(
            f,
            ". Proxy x1.5 is {} ({}), reported, not gated. Cortex-M4 proxy prices until measured on hardware.",
            margin,
            margin.share_of_whole_tick()
        )
    }
}
