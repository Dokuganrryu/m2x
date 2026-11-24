use super::trace::{Context, Event};

const GPIO_W: u32 = 0x4008_D000;
const GPIO_W_PORT_STEP: u32 = 0x80;
const GPIO_W_PIN_STEP: u32 = 4;
const GPIO_DIRSET: u32 = 0x4008_E380;
const GPIO_DIRSET_PORT_STEP: u32 = 4;
const PORTS: u32 = 2;
const PINS: u32 = 32;
const QUIESCE_LOWS: [(u32, u32); 4] = [(0, 28), (1, 26), (1, 22), (1, 10)];
const ORDER: &str = "the Quiesce script drives P0_28, P1_26, P1_22 and P1_10 low in that order";

fn level_word(address: u32) -> Option<(u32, u32)> {
    let offset = address.checked_sub(GPIO_W)?;
    let (port, pin) = (
        offset / GPIO_W_PORT_STEP,
        offset % GPIO_W_PORT_STEP / GPIO_W_PIN_STEP,
    );
    (port < PORTS && pin < PINS && offset % GPIO_W_PIN_STEP == 0).then_some((port, pin))
}

fn direction_set_port(address: u32) -> Option<u32> {
    let offset = address.checked_sub(GPIO_DIRSET)?;
    (offset < PORTS * GPIO_DIRSET_PORT_STEP && offset % GPIO_DIRSET_PORT_STEP == 0)
        .then_some(offset / GPIO_DIRSET_PORT_STEP)
}

pub fn quiesce_lows(events: &[Event]) -> Result<(), String> {
    let mut level: Option<(u32, u32, bool)> = None;
    let mut driven = 0;
    for event in events {
        let Event::Write(write) = event else { continue };
        if write.context != Context::After {
            continue;
        }
        if let Some((port, pin)) = level_word(write.address) {
            level = Some((port, pin, write.value != 0));
            continue;
        }
        let Some((port, pin, high)) = level else {
            continue;
        };
        if direction_set_port(write.address) != Some(port) || write.value >> pin & 1 == 0 {
            continue;
        }
        level = None;
        if high || QUIESCE_LOWS[driven] != (port, pin) {
            return Err(format!(
                "past `main` pin drive {} is P{port}_{pin} {} at step {}, and {ORDER}",
                driven + 1,
                if high { "high" } else { "low" },
                write.index
            ));
        }
        driven += 1;
        if driven == QUIESCE_LOWS.len() {
            return Ok(());
        }
    }
    let seen: Vec<String> = QUIESCE_LOWS[..driven]
        .iter()
        .map(|(port, pin)| format!("P{port}_{pin}"))
        .collect();
    Err(match seen.as_slice() {
        [] => format!("past `main` the image drives no pin, and {ORDER}"),
        seen => format!(
            "past `main` the image drives {} low and then stops driving pins, and {ORDER}",
            seen.join(", ")
        ),
    })
}
