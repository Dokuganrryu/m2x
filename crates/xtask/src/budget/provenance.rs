use super::attribution::{Attribution, attribute};
use super::program::{PricedTrace, Program};
use super::run::{Monitoring, Run};
use crate::price::Block;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Provenance {
    ImageCode,
    DiffersFromImage(Vec<String>),
    AbsentFromImage(Vec<String>),
    BenchGlue { callees_not_image_code: Vec<String> },
    NotMeasured,
}

pub fn same_code(bench: &Program, image: &Program, function: &str) -> Option<bool> {
    let image_shape = image.executed_shape_of(function)?;
    Some(bench.executed_shape_of(function) == Some(image_shape))
}

pub fn provenance_by_block(
    traces: &[(Run, PricedTrace)],
    bench: &Program,
    image: &Program,
) -> BTreeMap<Block, Provenance> {
    let mut functions: BTreeMap<Block, BTreeSet<String>> = BTreeMap::new();
    for (run, priced) in traces {
        for function in priced.per_function.keys() {
            let block = match run {
                Run::BenchTick(tick) => match attribute(function, tick.phase) {
                    Attribution::Dsp(block) if tick.monitoring == Monitoring::Off => Some(block),
                    Attribution::Dsp(_) => None,
                    Attribution::TickContext => Some(Block::TickContext),
                    Attribution::DmaMoves => Some(Block::DmaMoves),
                    Attribution::MonitoredKernel | Attribution::Unattributed => None,
                },
                Run::BenchUsb(_, path) => Some(Block::Usb(*path)),
                Run::ImageTick(_) | Run::ImageUsbIdle => None,
            };
            if let Some(block) = block {
                functions.entry(block).or_default().insert(function.clone());
            }
        }
    }
    Block::ALL
        .iter()
        .map(|block| {
            let provenance = match functions.get(block) {
                None => Provenance::NotMeasured,
                Some(names) => judge(names, bench, image),
            };
            (*block, provenance)
        })
        .collect()
}

fn judge(names: &BTreeSet<String>, bench: &Program, image: &Program) -> Provenance {
    let mut absent = Vec::new();
    let mut differs = Vec::new();
    let mut glue = false;
    for name in names {
        if name.starts_with("budget_") {
            glue = true;
            continue;
        }
        match same_code(bench, image, name) {
            None => absent.push(name.clone()),
            Some(false) => differs.push(name.clone()),
            Some(true) => {}
        }
    }
    if glue {
        Provenance::BenchGlue {
            callees_not_image_code: absent.into_iter().chain(differs).collect(),
        }
    } else if !absent.is_empty() {
        Provenance::AbsentFromImage(absent)
    } else if !differs.is_empty() {
        Provenance::DiffersFromImage(differs)
    } else {
        Provenance::ImageCode
    }
}

impl fmt::Display for Provenance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Provenance::ImageCode => f.write_str("image code, identical in the image"),
            Provenance::DiffersFromImage(names) => {
                write!(
                    f,
                    "bench copy, differs from the image in {}",
                    names.join(", ")
                )
            }
            Provenance::AbsentFromImage(names) => {
                write!(f, "bench only, not in the image: {}", names.join(", "))
            }
            Provenance::BenchGlue {
                callees_not_image_code,
            } if callees_not_image_code.is_empty() => f.write_str(
                "bench copy of the handler body, every function it calls is identical in the image",
            ),
            Provenance::BenchGlue {
                callees_not_image_code,
            } => write!(
                f,
                "bench copy of the handler body, calls functions not identical in the image: {}",
                callees_not_image_code.join(", ")
            ),
            Provenance::NotMeasured => f.write_str("not measured"),
        }
    }
}
