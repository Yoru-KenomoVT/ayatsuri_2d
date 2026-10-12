// The motion3 round trip: read a real motion3.json, write it back with our
// writer, read that again, and compare every curve at every frame.
//
//     cargo run --release -p ayatsuri --example motionround -- some.motion3.json

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("usage: motionround <file.motion3.json>")?;
    let bytes = std::fs::read(&path)?;
    let a = ayatsuri_import::from_motion3(&bytes, "in")?;
    let out = ayatsuri_import::to_motion3(&a);
    let b = ayatsuri_import::from_motion3(&out, "out")?;
    let keys: usize = a.curves.iter().map(|c| c.keys.len()).sum();
    println!(
        "{}: {} curves, {} keys, {:.2}s at {} fps, restricted beziers: {}",
        path,
        a.curves.len(),
        keys,
        a.duration,
        a.fps,
        a.restricted_beziers
    );
    let frames = (a.duration * a.fps).ceil() as usize;
    let mut worst = 0.0f32;
    for (ca, cb) in a.curves.iter().zip(&b.curves) {
        for f in 0..=frames {
            let t = f as f32 / a.fps;
            let x = ayatsuri_core::anim::sample(&a, ca, t).unwrap_or(0.0);
            let y = ayatsuri_core::anim::sample(&b, cb, t).unwrap_or(0.0);
            worst = worst.max((x - y).abs());
        }
    }
    println!("worst value difference over {} frames: {worst}", frames + 1);
    if a.curves.len() != b.curves.len() || worst > 1e-4 {
        return Err("MOTION ROUND TRIP DRIFTED".into());
    }
    println!("MOTION ROUND TRIP EXACT");
    Ok(())
}
