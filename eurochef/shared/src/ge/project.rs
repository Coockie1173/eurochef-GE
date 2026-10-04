//! A new map from a glTF scene: one level file with geometry, collision, textures and the spawn
//! point, ready for the game.

use std::path::{Path, PathBuf};

use anyhow::Context;

use super::{
    geomap::{build_geometry_file, BuildStats, GeScene},
    gltf_import::{import_gltf, ImportOptions},
    level::{level_hash, make_level_of_geometry},
};

#[derive(Clone, Debug)]
pub struct NewMapOptions {
    /// Goes into the file name: mt_NAME.edb
    pub name: String,
    /// Which of the new levels this is, its file's hash follows from it
    pub id: u32,
    /// Where the player starts. None: the scene's own spawn point, or its floor in the middle
    pub spawn: Option<[f32; 3]>,
    pub import: ImportOptions,
}

impl Default for NewMapOptions {
    fn default() -> Self {
        Self {
            name: "custom".to_string(),
            id: 1,
            spawn: None,
            import: ImportOptions::default(),
        }
    }
}

pub struct NewMap {
    pub file_name: String,
    pub level_hash: u32,
    pub data: Vec<u8>,
    pub spawn: [f32; 3],
    pub stats: BuildStats,
}

impl NewMap {
    /// Writes the level's file into a folder and returns its path
    pub fn save<P: AsRef<Path>>(&self, folder: P) -> anyhow::Result<PathBuf> {
        let folder = folder.as_ref();
        std::fs::create_dir_all(folder).with_context(|| format!("couldn't create {}", folder.display()))?;
        let path = folder.join(&self.file_name);
        std::fs::write(&path, &self.data).with_context(|| format!("couldn't write {}", path.display()))?;
        Ok(path)
    }
}

fn file_name_part(name: &str) -> String {
    let cleaned: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect();
    if cleaned.is_empty() {
        "custom".to_string()
    } else {
        cleaned
    }
}

pub fn new_map_from_scene(scene: &GeScene, options: &NewMapOptions) -> anyhow::Result<NewMap> {
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0);
    let level_hash = level_hash(options.id);
    let spawn = options.spawn.unwrap_or_else(|| scene.default_spawn());

    let (geometry, stats) = build_geometry_file(scene, level_hash, time);
    let data = make_level_of_geometry(&geometry, level_hash, spawn).context("couldn't add the spawn point")?;

    Ok(NewMap {
        file_name: format!("mt_{}.edb", file_name_part(&options.name)),
        level_hash,
        data,
        spawn,
        stats,
    })
}

pub fn new_map_from_gltf<P: AsRef<Path>>(gltf: P, options: &NewMapOptions) -> anyhow::Result<NewMap> {
    let scene = import_gltf(gltf, &options.import)?;
    new_map_from_scene(&scene, options)
}
