use crate::adc::{AdcAddress, AdcWrite};
use crate::analog_settings::AnalogSettings;
use dsp::{FramesPerTick, RateFamily};

use crate::analog_word::AnalogWord;
use crate::breadcrumb::{At, Weight};
use crate::dac::{DacWrite, StatusRegister};
use crate::expander::ExpanderOutputs;
use crate::fault::FaultCode;
use crate::input::Input;
use crate::output_pin::OutputPin;
use crate::phantom::{self, PinLevel};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    DrivePin(OutputPin, PinLevel),
    SendAnalogWord(AnalogWord),
    WriteDac(DacWrite),
    ReadDac(StatusRegister),
    ProbeAdc(AdcAddress),
    WriteAdc(AdcWrite),
    SwitchPll0Family(RateFamily),
    StartI2s(FramesPerTick),
    StopI2s,
    StartDma,
    StopDma,
    StartTickTimer(FramesPerTick),
    StopTickTimer,
    MuteInput(Input),
    UnmuteInput(Input),
    AttachUsb,
    DisableInterrupts,
    DetachUsb,
    FeedWatchdog,
    ArmWatchdog,
    FenceFlashController,
    ConfigureAnalogWordBus,
    StartClockInput,
    ReadClockInput,
    StartCorePll,
    SelectCorePllAsMainClock,
    EnableMasterClockOutput,
    ConfigureConverterBus,
    WriteExpanderDirection,
    ReadExpanderInputs,
    WriteExpanderOutputs(ExpanderOutputs),
    StartKnobScan,
    ZeroTransmitBuffers,
    StopTransmitDma,
    LatchPlaybackSilence,
    RecordFault(FaultCode, At, Weight),
}

impl Command {
    pub const fn from_phantom(output: phantom::Output, analog: AnalogSettings) -> Self {
        match output {
            phantom::Output::MuteInput(input) => Command::MuteInput(input),
            phantom::Output::UnmuteInput(input) => Command::UnmuteInput(input),
            phantom::Output::SendAnalogWord(grant) => Command::SendAnalogWord(analog.word(grant)),
            phantom::Output::DrivePhantomActivePin(level) => {
                Command::DrivePin(OutputPin::PhantomActive, level)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandListFull;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandList<const CAPACITY: usize> {
    items: [Command; CAPACITY],
    len: usize,
}

impl<const CAPACITY: usize> CommandList<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            items: [Command::StopI2s; CAPACITY],
            len: 0,
        }
    }

    pub fn as_slice(&self) -> &[Command] {
        &self.items[..self.len]
    }

    pub fn push(&mut self, command: Command) -> Result<(), CommandListFull> {
        let slot = self.items.get_mut(self.len).ok_or(CommandListFull)?;
        *slot = command;
        self.len += 1;
        Ok(())
    }

    pub fn extend(
        &mut self,
        commands: impl IntoIterator<Item = Command>,
    ) -> Result<(), CommandListFull> {
        commands
            .into_iter()
            .try_for_each(|command| self.push(command))
    }
}

impl<const CAPACITY: usize> Default for CommandList<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}
