//! `cargo run -p ayatsuri-pen --example probe [seconds]`: what the reader
//! finds, then every pressure change for a while, to check a tablet.

fn main() {
    let secs: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(5);
    ayatsuri_pen::start(|| {});
    std::thread::sleep(std::time::Duration::from_millis(300));
    println!("status: {:?}", ayatsuri_pen::status());
    let until = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    let mut last = None;
    while std::time::Instant::now() < until {
        let now = ayatsuri_pen::reading();
        if now != last {
            println!("{now:?}");
            last = now;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
