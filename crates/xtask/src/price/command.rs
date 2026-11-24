use super::price_listing;
use crate::exit::Outcome;
use std::fs;

pub fn command(listings: &[String]) -> Outcome {
    let mut priced = 0;
    let mut failed = 0;
    for path in listings {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) => {
                eprintln!("{path}: {error}");
                return Outcome::UsageOrInputError;
            }
        };
        let prices = match price_listing(&text) {
            Ok(prices) => prices,
            Err(error) => {
                eprintln!("{path}: {error}");
                return Outcome::UsageOrInputError;
            }
        };
        for price in prices {
            println!("{price}");
            priced += 1;
            if !price.passes() {
                failed += 1;
            }
        }
    }
    println!(
        "{priced} blocks priced, {failed} outside their D3 share. Cortex-M4 proxy prices until measured on hardware."
    );
    if failed == 0 {
        Outcome::GatePassed
    } else {
        Outcome::GateFailed
    }
}
