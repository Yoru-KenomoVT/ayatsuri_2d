//! Write the fixture model to a file: `cargo run -p ayatsuri-capi --features
//! fixture --example small_aom -- out.aom`. The C host example drives it.

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "small.aom".into());
    let bytes = ayatsuri_capi::fixture::small_aom();
    if let Err(e) = std::fs::write(&out, &bytes) {
        eprintln!("{out}: {e}");
        std::process::exit(1);
    }
    println!("wrote {out} ({} bytes)", bytes.len());
}
