// Headless physics smoke test: load a model with its physics3.json, swing
// the head for a couple of seconds and watch a driven parameter move.
//
//     cargo run --release -p ayatsuri --example physics -- model.moc3

#![forbid(unsafe_code)]

fn puppet_model_clone(p: &ayatsuri_core::Puppet) -> ayatsuri_format::Model {
    p.model().clone()
}

fn main() -> Result<(), String> {
    let moc_path = std::env::args().nth(1).ok_or("usage: physics <model.moc3> [physics3.json]")?;
    let phys_path = std::env::args()
        .nth(2)
        .unwrap_or_else(|| moc_path.trim_end_matches(".moc3").to_string() + ".physics3.json");

    let bytes = std::fs::read(&moc_path).map_err(|e| format!("{moc_path}: {e}"))?;
    let mut model = ayatsuri_import::from_moc3(&bytes).map_err(|e| e.to_string())?;
    let pb = std::fs::read(&phys_path).map_err(|e| format!("{phys_path}: {e}"))?;
    let rig = ayatsuri_import::from_physics3(&pb).map_err(|e| e.to_string())?;
    println!("rig: {} groups, fps {}", rig.groups.len(), rig.fps);

    // the first output of the first group is the parameter we watch
    let watch_id = rig
        .groups
        .iter()
        .flat_map(|g| g.outputs.iter())
        .map(|o| o.param.clone())
        .next()
        .ok_or("rig has no outputs")?;
    model.physics = Some(rig);

    let angle = model.parameters.iter().position(|p| p.id == "ParamAngleX");
    let watch = model
        .parameters
        .iter()
        .position(|p| p.id == watch_id)
        .ok_or(format!("{watch_id} not in model"))?;
    println!("driving ParamAngleX ({angle:?}), watching {watch_id} ({watch})");

    let mut puppet = ayatsuri_core::Puppet::new(model);
    if !puppet.has_physics() {
        return Err("puppet built without physics".into());
    }
    // idle first: nothing drives anything, nothing should move
    {
        let mut idle = ayatsuri_core::Puppet::new(puppet_model_clone(&puppet));
        for _ in 0..600 {
            idle.step_physics(1.0 / 60.0);
        }
        let mut worst = (0usize, 0.0f32);
        for (i, p) in idle.model().parameters.iter().enumerate() {
            let drift = (idle.value(i) - p.default).abs();
            if drift > worst.1 {
                worst = (i, drift);
            }
        }
        let id = idle.model().parameters.get(worst.0).map(|p| p.id.clone()).unwrap_or_default();
        println!("idle 10s: worst drift {id} = {:.4}", worst.1);
        if worst.1 > 0.01 {
            return Err(format!("idle drift on {id}: {}", worst.1));
        }
    }

    let dt = 1.0 / 60.0;
    let mut peak = 0.0f32;
    let mut moved = false;
    for frame in 0..360 {
        let t = frame as f32 * dt;
        // one nod in the first second, then hold still and watch it settle
        if let Some(a) = angle {
            let drive = if t < 1.0 { (t * core::f32::consts::TAU).sin() * 15.0 } else { 0.0 };
            puppet.set_value(a, drive);
        }
        if puppet.step_physics(dt) {
            moved = true;
        }
        puppet.update();
        let v = puppet.value(watch);
        peak = peak.max(v.abs());
        if frame % 12 == 0 {
            println!("t={t:4.2}s  {watch_id} = {v:+.4}");
        }
    }
    // the self-feedback rigs (AngleX writes AngleX) must decay, not stick
    let residual = angle.map(|a| puppet.value(a).abs()).unwrap_or(0.0);
    println!("residual ParamAngleX after settling: {residual:.4}");
    if residual > 0.5 {
        return Err(format!("self-feedback did not decay (AngleX = {residual})"));
    }
    if moved && peak > 1e-3 {
        println!("PHYSICS OK (peak |{watch_id}| = {peak:.4})");
        Ok(())
    } else {
        Err(format!("physics never moved {watch_id} (peak {peak})"))
    }
}
