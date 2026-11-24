#![no_std]
#![forbid(unsafe_code)]

pub mod adc;
pub mod analog_settings;
pub mod analog_word;
pub mod boot;
pub mod breadcrumb;
pub mod command;
pub mod console;
pub mod control_pass;
pub mod core_clock;
pub mod cycles;
pub mod dac;
pub mod expander;
pub mod factory;
pub mod factory_init;
pub mod fault;
pub mod i2c;
pub mod input;
pub mod knob;
pub mod knob_law;
pub mod knob_scan;
pub mod lcd;
mod monitor;
pub mod output_pin;
pub mod outputs;
pub mod panel_bus;
pub mod phantom;
pub mod pin;
pub mod pll;
#[cfg(feature = "sound2")]
pub mod preset;
pub mod quiesce;
pub mod rate_change;
#[cfg(feature = "register-zero-protocol")]
pub mod register_zero;
#[cfg(doctest)]
pub mod register_zero_proofs;
pub mod safe_init;
pub mod serial_bus;
pub mod silent;
pub mod stage_batch;
pub mod steps;
pub mod supervisor;
pub mod usb;
pub mod usb_detach;
mod volume_slew;
