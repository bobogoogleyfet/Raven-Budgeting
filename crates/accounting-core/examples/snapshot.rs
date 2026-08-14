//! Regenerate the rate table bundled with the app.
//!
//! The snapshot exists so a machine that has never been online can still tax a
//! receipt (see [`accounting_core::rates::bundled`]). It is refreshed by hand,
//! rarely — a running install updates itself over the network, and this is only
//! the floor a brand-new one starts from.
//!
//! Producing it *through the parser* is the point of this file. A snapshot
//! transcribed by any other route could disagree with what a download installs,
//! and the two would drift in ways nothing would catch.
//!
//! ```sh
//! curl -O https://files.tax.utah.gov/tax/salestax/rate/26q3combined.xlsx
//! cargo run -p accounting-core --example snapshot -- 26q3combined.xlsx 2026Q3 \
//!     > crates/accounting-core/src/rates/utah_2026q3.csv
//! ```
//!
//! Then point `BUNDLED_CSV` at the new file and bump `BUNDLED_QUARTER`.

use std::process::ExitCode;

use accounting_core::rates::parse_workbook;
use accounting_core::time::Quarter;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [path, quarter] = args.as_slice() else {
        eprintln!("usage: snapshot <combined-rates.xlsx> <YYYYQn>");
        return ExitCode::FAILURE;
    };

    let Some(quarter) = parse_quarter(quarter) else {
        eprintln!("{quarter} is not a quarter like 2026Q3");
        return ExitCode::FAILURE;
    };

    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("could not read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let table = match parse_workbook(&bytes, quarter) {
        Ok(table) => table,
        Err(e) => {
            eprintln!("could not read the workbook: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Written into the file so the next person to open it can tell what it is
    // and where it came from without going back to the commit message.
    println!("# Utah State Tax Commission — combined sales and use tax rates");
    println!("# quarter: {quarter}");
    match table.effective_on {
        Some(date) => println!("# in effect: {date}"),
        None => println!("# in effect: {} (from the quarter)", quarter.effective_on()),
    }
    println!(
        "# source: {}",
        accounting_core::rates::workbook_url(quarter)
    );
    println!("# regenerate: cargo run -p accounting-core --example snapshot");
    println!("code,name,combined_rate_ppm,food_rate_ppm");
    for area in &table.areas {
        // The name can hold a comma ("St. George" cannot, but nothing promises
        // the next annexation is as polite), so it is quoted whenever it needs
        // to be and never otherwise — a diff of this file should be readable.
        let name = if area.name.contains([',', '"']) {
            format!("\"{}\"", area.name.replace('"', "\"\""))
        } else {
            area.name.clone()
        };
        println!(
            "{},{},{},{}",
            area.code,
            name,
            area.combined_rate_ppm,
            area.food_rate_ppm
                .map_or_else(String::new, |ppm| ppm.to_string())
        );
    }
    eprintln!("{} jurisdictions", table.areas.len());
    ExitCode::SUCCESS
}

/// `"2026Q3"` → the quarter.
fn parse_quarter(text: &str) -> Option<Quarter> {
    let (year, quarter) = text.split_once(['Q', 'q'])?;
    Quarter::new(year.trim().parse().ok()?, quarter.trim().parse().ok()?)
}
