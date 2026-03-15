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

    let mesh = cs_flythrough::bsp::load(&bsp_path, &install)
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
