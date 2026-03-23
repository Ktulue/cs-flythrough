use std::path::PathBuf;

fn cs_install_path() -> Option<PathBuf> {
    std::env::var("CS_INSTALL_PATH")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            let fallback = PathBuf::from(
                "C:/Program Files (x86)/Steam/steamapps/common/Counter-Strike",
            );
            fallback.exists().then_some(fallback)
        })
        .or_else(|| {
            let fallback = PathBuf::from(
                "C:/Program Files (x86)/Steam/steamapps/common/Condition Zero",
            );
            fallback.exists().then_some(fallback)
        })
}

#[test]
fn test_de_dust2_loads() {
    let Some(install) = cs_install_path() else {
        eprintln!(
            "Skipping integration test: CS install not found. Set CS_INSTALL_PATH env var."
        );
        return;
    };

    let bsp_path = cs_flythrough::maplist::resolve_bsp(&install, "de_dust2")
        .expect("de_dust2.bsp not found in CS install");

    let (mesh, _collision) = cs_flythrough::bsp::load(&bsp_path, &install)
        .expect("BSP load failed");

    assert!(!mesh.vertices.is_empty(), "no vertices");
    assert!(!mesh.indices.is_empty(), "no indices");
    assert!(mesh.entity_origins.len() >= 4, "fewer than 4 entity origins");
    assert!(
        mesh.sky_index_offset <= mesh.indices.len() as u32,
        "sky_index_offset out of range"
    );
    println!(
        "de_dust2: {} vertices, {} indices, {} waypoints, sky_offset={}",
        mesh.vertices.len(),
        mesh.indices.len(),
        mesh.entity_origins.len(),
        mesh.sky_index_offset,
    );
}

#[test]
fn collision_known_positions() {
    let install = match cs_install_path() {
        Some(p) => p,
        None => { eprintln!("skipping: CS install not found"); return; }
    };
    let bsp = install.join("cstrike/maps/de_dust2.bsp");
    if !bsp.exists() {
        eprintln!("skipping: de_dust2.bsp not found");
        return;
    }

    let (_mesh, collision) = cs_flythrough::bsp::load(&bsp, &install).expect("bsp::load failed");

    use cs_flythrough::bsp::collision::point_in_solid;
    use glam::Vec3;

    // CT spawn area (standing height) — should be open air.
    // Origin (448, 2464, -88) is the first CT spawn entity origin in de_dust2.
    assert!(
        !point_in_solid(&collision, Vec3::new(448.0, 2464.0, -88.0)),
        "CT spawn origin should be open"
    );

    // Below the CT spawn floor — should be solid ground.
    assert!(
        point_in_solid(&collision, Vec3::new(448.0, 2464.0, -188.0)),
        "below CT spawn floor should be solid"
    );
}
