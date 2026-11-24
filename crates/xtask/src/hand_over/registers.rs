use std::collections::BTreeMap;

// WHY: CMSIS core_cm33.h, SysTick at 0xE000E010, NVIC at 0xE000E100, SCB at 0xE000ED00, MPU at 0xE000ED90
const CORE_REGISTERS: [(u32, &str); 21] = [
    (0xE000_ED88, "SCB CPACR"),
    (0xE000_E010, "SysTick CSR"),
    (0xE000_E014, "SysTick RVR"),
    (0xE000_E018, "SysTick CVR"),
    (0xE000_E100, "NVIC ISER0"),
    (0xE000_E104, "NVIC ISER1"),
    (0xE000_E180, "NVIC ICER0"),
    (0xE000_E184, "NVIC ICER1"),
    (0xE000_E280, "NVIC ICPR0"),
    (0xE000_E284, "NVIC ICPR1"),
    (0xE000_ED04, "SCB ICSR"),
    (0xE000_ED08, "SCB VTOR"),
    (0xE000_ED0C, "SCB AIRCR"),
    (0xE000_ED14, "SCB CCR"),
    (0xE000_ED28, "SCB CFSR"),
    (0xE000_ED94, "MPU CTRL"),
    (0xE000_ED98, "MPU RNR"),
    (0xE000_ED9C, "MPU RBAR"),
    (0xE000_EDA0, "MPU RLAR"),
    (0xE000_EDC0, "MPU MAIR0"),
    (0xE000_EDC4, "MPU MAIR1"),
];

pub struct RegisterNames(BTreeMap<u32, String>);

fn number(text: &str) -> Option<u32> {
    let text = text.trim_end_matches(|c: char| !c.is_ascii_hexdigit());
    match text.strip_prefix("0x") {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}

fn dimensions(line: &str) -> Vec<(u32, u32)> {
    let words: Vec<&str> = line
        .split(|c: char| c.is_whitespace() || c == '(')
        .collect();
    words
        .windows(4)
        .filter(|w| w[0] == "dim" && w[2] == "step")
        .filter_map(|w| Some((number(w[1])?, number(w[3])?)))
        .collect()
}

impl RegisterNames {
    pub fn from_rows(rows: &str) -> Self {
        let mut names = BTreeMap::new();
        for (address, name) in CORE_REGISTERS {
            names.insert(address, name.to_string());
        }
        for line in rows.lines() {
            let words: Vec<&str> = line.split_whitespace().collect();
            let ["REG", peripheral, register, ..] = words.as_slice() else {
                continue;
            };
            let Some(base) = words
                .windows(2)
                .find(|pair| pair[1] == "offset")
                .and_then(|pair| number(pair[0]))
            else {
                continue;
            };
            let array = register.split('(').next().unwrap_or(register);
            let member = line
                .find(").")
                .and_then(|close| line[close + 1..].split_whitespace().next())
                .unwrap_or("");
            let name = format!("{peripheral} {array}{member}");
            let mut addresses = vec![(base, name.clone())];
            for (count, step) in dimensions(line) {
                addresses = addresses
                    .into_iter()
                    .flat_map(|(address, name)| {
                        (0..count).map(move |index| {
                            (
                                address + index * step,
                                name.replacen("%s", &index.to_string(), 1),
                            )
                        })
                    })
                    .collect();
            }
            for (address, name) in addresses {
                names.entry(address).or_insert(name);
            }
        }
        Self(names)
    }

    pub fn name(&self, address: u32) -> &str {
        self.0.get(&address).map_or("unnamed", String::as_str)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Starts {
    AnyWrite,
    WhenBitSet(u32),
    WhenNonZero,
}

// WHY: SVD28 names in fixtures/lpc55s28-svd-registers.txt; CTIMER TCR CEN bit 0, I2S CFG1 MAINENABLE bit 0, USBHSD DEVCMDSTAT DCON bit 16, a GPIO word pin written nonzero drives the pin high
const BRING_UP: [(&str, Starts); 19] = [
    ("SYSCON PLL", Starts::AnyWrite),
    ("SYSCON MAINCLKSEL", Starts::AnyWrite),
    ("SYSCON MCLK", Starts::AnyWrite),
    ("ANACTRL XO32M_CTRL", Starts::AnyWrite),
    ("PMC", Starts::AnyWrite),
    ("USBPHY", Starts::AnyWrite),
    ("DMA0 ENABLESET0", Starts::AnyWrite),
    ("DMA0 SETTRIG0", Starts::AnyWrite),
    ("CTIMER0 TCR", Starts::WhenBitSet(1)),
    ("I2S", Starts::WhenBitSet(1)),
    ("USBHSD DEVCMDSTAT", Starts::WhenBitSet(1 << 16)),
    ("FLEXCOMM1", Starts::AnyWrite),
    ("SPI1", Starts::AnyWrite),
    ("FLEXCOMM3", Starts::AnyWrite),
    ("I2C3", Starts::AnyWrite),
    ("FLEXCOMM4", Starts::AnyWrite),
    ("SPI4", Starts::AnyWrite),
    ("ADC0", Starts::AnyWrite),
    ("GPIO W", Starts::WhenNonZero),
];

pub fn starts_something(name: &str, value: u32) -> bool {
    BRING_UP.iter().any(|(prefix, starts)| {
        name.starts_with(prefix)
            && match starts {
                Starts::AnyWrite => true,
                Starts::WhenBitSet(bit) => value & bit != 0,
                Starts::WhenNonZero => value != 0,
            }
    })
}
