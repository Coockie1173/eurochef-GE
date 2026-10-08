use std::{
    collections::hash_map,
    fs::File,
    io::{BufReader, Cursor, Read, Seek},
    sync::Arc,
};

use crossbeam::atomic::AtomicCell;
use eframe::CreationContext;
use egui::{mutex::RwLock, Color32, FontData, FontDefinitions, NumExt};
use eurochef_edb::{
    binrw::{BinReaderExt, Endian},
    edb::EdbFile,
    versions::Platform,
    Hashcode, HashcodeUtils,
};
use eurochef_shared::ge::sky::{self, SkyOptions};
use eurochef_shared::filesystem::path::DissectedFilelistPath;
use eurochef_shared::{
    hashcodes::parse_hashcodes, script::UXGeoScript, spreadsheets::UXGeoSpreadsheet,
    textures::UXGeoTexture,
};
use instant::Instant;
use nohash_hasher::IntMap;

use crate::{
    entities::{self},
    fileinfo, maps,
    render::{entity::EntityRenderer, RenderStore},
    scripts, spreadsheet, textures,
};

/// Basic app tracking state
pub enum AppState {
    Ready,
    SelectPlatform,
    Loading(String),
    Error(anyhow::Error),
}

#[derive(PartialEq)]
enum Panel {
    FileInfo,
    Maps,
    Entities,
    Textures,
    Spreadsheets,
    Scripts,
}

pub struct EurochefApp {
    gl: Arc<glow::Context>,

    state: AppState,
    current_panel: Panel,

    spreadsheetlist: Option<spreadsheet::TextItemList>,
    fileinfo: Option<fileinfo::FileInfoPanel>,
    textures: Option<textures::TextureList>,
    entities: Option<entities::EntityListPanel>,
    maps: Option<maps::MapViewerPanel>,
    scripts: Option<scripts::ScriptListPanel>,

    load_input: Arc<AtomicCell<Option<(Vec<u8>, String)>>>,
    pending_file: Option<(Vec<u8>, Option<Platform>)>,
    selected_platform: Platform,

    ps2_warning: bool,
    about_window: bool,
    show_profiler: bool,

    hashcodes: Arc<IntMap<u32, String>>,
    path_cache: IntMap<Hashcode, String>,
    render_store: Arc<RwLock<RenderStore>>,
    game: String,

    /// --screenshot: the file, the panel to show, frames left until it is taken
    screenshot: Option<(String, String, u32)>,
    /// --camera and --select: the map view's camera (x, y, z, pitch, yaw) and its selected trigger
    view: Option<([f32; 5], Option<usize>)>,
    /// --no-windows
    no_windows: bool,

    /// Where the pending file comes from, and the loaded file's path and bytes
    pending_path: String,
    current_source: Option<(String, Arc<Vec<u8>>)>,

    new_map: Option<NewMapDialog>,
    /// --save-triggers: write the loaded file's triggers back to this file (for testing)
    save_triggers_to: Option<String>,
}

/// File > New GoldenEye 007 map: a level made from a glTF scene
struct NewMapDialog {
    gltf: String,
    folder: String,
    name: String,
    id: u32,
    scale: f32,
    bake_light: bool,
    brightness: f32,
    /// A made sky's preset. Empty: the scene's own sky, if it has one
    sky_preset: String,
    /// The made sky as it is set, a preset to begin with
    sky: SkyOptions,
    /// Where the sky's picture looks: degrees around and up
    sky_look: (f32, f32),
    /// The sky's picture, None when it has to be drawn again
    sky_picture: Option<egui::TextureHandle>,
    status: String,
}

impl Default for NewMapDialog {
    fn default() -> Self {
        Self {
            gltf: String::new(),
            folder: String::new(),
            name: "custom".to_string(),
            id: 1,
            scale: 1.0,
            bake_light: true,
            brightness: 1.0,
            sky_preset: String::new(),
            sky: SkyOptions::preset("day").expect("the day preset"),
            sky_look: (0.0, 20.0),
            sky_picture: None,
            status: String::new(),
        }
    }
}

impl EurochefApp {
    /// Called once before the first frame.
    pub fn new(
        path: Option<String>,
        hashcodes_path: Option<String>,
        cc: &CreationContext<'_>,
    ) -> Self {
        // Install FontAwesome font and place it second
        let mut fonts = FontDefinitions::default();
        fonts.font_data.insert(
            "font_awesome".to_owned(),
            FontData::from_static(include_bytes!("../assets/FontAwesomeSolid.ttf")),
        );

        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(1, "font_awesome".to_owned());

        cc.egui_ctx.set_fonts(fonts);

        #[cfg(not(any(target_arch = "wasm32", target_os = "macos")))]
        unsafe {
            use glow::HasContext;
            let gl = cc.gl.as_ref().unwrap();

            // The debug callback needs exclusive access to the context, which eframe shares
            gl.enable(glow::DEBUG_OUTPUT);
            gl.debug_message_control(glow::DONT_CARE, glow::DONT_CARE, glow::DONT_CARE, &[], true);
        }

        let hashcodes = if let Some(hashcodes_path) = hashcodes_path {
            let hfs = std::fs::read_to_string(hashcodes_path).unwrap();
            parse_hashcodes(&hfs)
        } else {
            Default::default()
        };

        let mut s = Self {
            gl: cc.gl.clone().unwrap(),
            state: AppState::Ready,
            current_panel: Panel::FileInfo,
            spreadsheetlist: None,
            fileinfo: None,
            textures: None,
            entities: None,
            maps: None,
            scripts: None,
            load_input: Arc::new(AtomicCell::new(None)),
            pending_file: None,
            selected_platform: Platform::Ps2,
            ps2_warning: false,
            about_window: false,
            path_cache: Default::default(),
            render_store: Arc::new(RwLock::new(RenderStore::new())),
            hashcodes: Arc::new(hashcodes),
            game: String::new(),
            show_profiler: false,
            screenshot: None,
            view: None,
            no_windows: false,
            pending_path: String::new(),
            current_source: None,
            new_map: None,
            save_triggers_to: None,
        };

        if let Some(path) = path {
            match s.load_file_with_path(path) {
                Ok(_) => {}
                Err(e) => {
                    s.state = AppState::Error(e);
                }
            }
        }

        s
    }

    pub fn save_triggers_request(&mut self, path: String) {
        self.save_triggers_to = Some(path);
    }

    /// The window of File > New GoldenEye 007 map
    /// The made sky's own settings: a picture of it to look around in, its colours, its clouds
    /// and where the sphere is
    fn show_sky_settings(ui: &mut egui::Ui, dialog: &mut NewMapDialog) {
        const PICTURE: (u32, u32) = (352, 198);
        egui::CollapsingHeader::new("Sky settings").default_open(true).show(ui, |ui| {
            let mut changed = false;
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    if dialog.sky_picture.is_none() {
                        match sky::preview(&dialog.sky, PICTURE, dialog.sky_look.0, dialog.sky_look.1) {
                            Ok(image) => {
                                let image = egui::ColorImage::from_rgba_unmultiplied(
                                    [PICTURE.0 as usize, PICTURE.1 as usize],
                                    image.as_raw(),
                                );
                                dialog.sky_picture = Some(ui.ctx().load_texture("new_map_sky", image, Default::default()));
                            }
                            Err(e) => {
                                ui.colored_label(egui::Color32::LIGHT_RED, format!("{e:#}"));
                            }
                        }
                    }
                    if let Some(picture) = &dialog.sky_picture {
                        let response = ui.add(egui::Image::new(picture).sense(egui::Sense::drag()));
                        let by = response.drag_delta();
                        if by != egui::Vec2::ZERO {
                            dialog.sky_look.0 = (dialog.sky_look.0 - by.x * 0.25).rem_euclid(360.0);
                            dialog.sky_look.1 = (dialog.sky_look.1 + by.y * 0.25).clamp(-89.0, 89.0);
                            changed = true;
                        }
                    }
                    ui.weak("What a player sees of it. Drag to look around.");
                });
                egui::Grid::new("new_map_sky_settings").num_columns(2).show(ui, |ui| {
                    let sky = &mut dialog.sky;
                    let painted = sky.texture.is_none();
                    let mut colour = |ui: &mut egui::Ui, label: &str, colour: &mut sky::Colour, enabled: bool| {
                        ui.label(label);
                        // the colours are the way they look, as the picker's bytes are
                        let mut bytes = colour.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
                        let edited = ui.add_enabled_ui(enabled, |ui| ui.color_edit_button_srgb(&mut bytes).changed()).inner;
                        if edited {
                            *colour = bytes.map(|b| b as f32 / 255.0);
                        }
                        ui.end_row();
                        edited
                    };
                    changed |= colour(ui, "Zenith", &mut sky.zenith, painted);
                    changed |= colour(ui, "Horizon", &mut sky.horizon, painted);
                    changed |= colour(ui, "Ground", &mut sky.ground, painted);
                    changed |= colour(ui, "Clouds", &mut sky.cloud_colour, painted);
                    let mut slider = |ui: &mut egui::Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, hint: &str| {
                        ui.label(label).on_hover_text(hint);
                        let moved = ui.add_enabled(painted, egui::Slider::new(value, range)).on_hover_text(hint).changed();
                        ui.end_row();
                        moved
                    };
                    changed |= slider(ui, "Horizon falloff", &mut sky.falloff, 0.2..=3.0, "How soon the horizon gives way to the zenith: below 1 soon, above 1 late");
                    changed |= slider(ui, "Cloud cover", &mut sky.clouds, 0.0..=1.0, "How much of the sky the clouds take");
                    changed |= slider(ui, "Cloud size", &mut sky.cloud_scale, 0.5..=10.0, "Larger: smaller clouds");
                    changed |= slider(ui, "Cloud softness", &mut sky.cloud_softness, 0.02..=0.6, "How soft their edges are");
                    changed |= slider(ui, "Cloud opacity", &mut sky.cloud_opacity, 0.0..=1.0, "How much of the sky they hide");
                    ui.label("Seed").on_hover_text("Another number, other clouds");
                    ui.horizontal(|ui| {
                        changed |= ui.add_enabled(painted, egui::DragValue::new(&mut sky.seed).range(1..=9999)).changed();
                        if ui.add_enabled(painted, egui::Button::new("Other clouds")).clicked() {
                            sky.seed = sky.seed % 9999 + 1;
                            changed = true;
                        }
                    });
                    ui.end_row();
                    ui.label("Detail").on_hover_text("Segments around the sphere: more is smoother clouds and more triangles");
                    changed |= ui.add(egui::Slider::new(&mut sky.segments, 16..=128).step_by(4.0)).changed();
                    ui.end_row();

                    ui.label("Panorama").on_hover_text("A picture of the whole sky unrolled (2:1) in place of the painted one");
                    ui.horizontal(|ui| {
                        match &sky.texture {
                            Some(path) => ui.label(path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()),
                            None => ui.weak("none, the sky is painted"),
                        };
                        if ui.button("Browse").clicked() {
                            if let Some(path) = rfd::FileDialog::new().add_filter("Picture", &["png", "jpg", "jpeg", "tga"]).pick_file() {
                                sky.texture = Some(path);
                                changed = true;
                            }
                        }
                        if sky.texture.is_some() && ui.button("Remove").clicked() {
                            sky.texture = None;
                            changed = true;
                        }
                    });
                    ui.end_row();

                    ui.label("Radius").on_hover_text("Of the sphere around the level");
                    ui.horizontal(|ui| {
                        let mut own = sky.radius.is_some();
                        if ui.checkbox(&mut own, "set by hand").changed() {
                            sky.radius = own.then_some(300.0);
                        }
                        match &mut sky.radius {
                            Some(radius) => {
                                ui.add(egui::DragValue::new(radius).speed(1.0).range(10.0..=5000.0));
                            }
                            None => {
                                ui.weak("from the level's size");
                            }
                        }
                    });
                    ui.end_row();
                });
            });
            if changed {
                dialog.sky_picture = None;
            }
            ui.horizontal(|ui| {
                ui.weak("The same from the command line:");
                if ui.small_button("Copy").clicked() {
                    ui.ctx().copy_text(Self::sky_command(&dialog.sky_preset, &dialog.sky));
                }
            });
            ui.add(egui::Label::new(egui::RichText::new(Self::sky_command(&dialog.sky_preset, &dialog.sky)).monospace().small()).wrap());
        });
    }

    /// The options of `eurochef-cli ge new-map` that make the sky as it is set
    fn sky_command(preset: &str, sky: &SkyOptions) -> String {
        let hex = |c: &sky::Colour| {
            let b = c.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
            format!("'#{:02x}{:02x}{:02x}'", b[0], b[1], b[2])
        };
        let mut words = vec![];
        match &sky.texture {
            Some(path) => words.push(format!("--sky-texture '{}'", path.display())),
            None => {
                words.push(format!(
                    "--sky {preset} --sky-zenith {} --sky-horizon {} --sky-ground {} --sky-falloff {:.2} --sky-clouds {:.2}",
                    hex(&sky.zenith),
                    hex(&sky.horizon),
                    hex(&sky.ground),
                    sky.falloff,
                    sky.clouds
                ));
                if sky.clouds > 0.0 {
                    words.push(format!(
                        "--sky-cloud-colour {} --sky-cloud-scale {:.2} --sky-cloud-softness {:.2} --sky-cloud-opacity {:.2} --sky-seed {}",
                        hex(&sky.cloud_colour),
                        sky.cloud_scale,
                        sky.cloud_softness,
                        sky.cloud_opacity,
                        sky.seed
                    ));
                }
            }
        }
        words.push(format!("--sky-segments {}", sky.segments));
        if let Some(radius) = sky.radius {
            words.push(format!("--sky-radius {radius}"));
        }
        words.join(" ")
    }

    fn show_new_map(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.new_map.as_mut() else {
            return;
        };
        let mut open = true;
        let mut create = false;
        egui::Window::new("New GoldenEye 007 map")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label("A level file (mt_NAME.edb) is made from a glTF scene: its triangles, its textures, the collision and the player's spawn point.");
                ui.add_space(4.0);
                egui::Grid::new("new_map").num_columns(3).show(ui, |ui| {
                    ui.label("Scene");
                    ui.add(egui::TextEdit::singleline(&mut dialog.gltf).desired_width(320.0).hint_text(".gltf or .glb"));
                    if ui.button("Browse").clicked() {
                        if let Some(path) = rfd::FileDialog::new().add_filter("glTF", &["gltf", "glb"]).pick_file() {
                            dialog.gltf = path.to_string_lossy().to_string();
                            if dialog.name == "custom" {
                                if let Some(stem) = path.file_stem() {
                                    dialog.name = stem.to_string_lossy().to_lowercase();
                                }
                            }
                        }
                    }
                    ui.end_row();

                    ui.label("Folder");
                    ui.add(egui::TextEdit::singleline(&mut dialog.folder).desired_width(320.0).hint_text("the game's mods folder"));
                    if ui.button("Browse").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                            dialog.folder = path.to_string_lossy().to_string();
                        }
                    }
                    ui.end_row();

                    ui.label("Name");
                    ui.text_edit_singleline(&mut dialog.name);
                    ui.end_row();

                    ui.label("Level number");
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut dialog.id).range(0..=0xDFF));
                        ui.label(format!(
                            "level {:08X}",
                            eurochef_shared::ge::level::level_hash(dialog.id)
                        ));
                    });
                    ui.end_row();

                    ui.label("Scale");
                    ui.add(egui::DragValue::new(&mut dialog.scale).speed(0.01).range(0.001..=1000.0));
                    ui.end_row();

                    ui.label("Light");
                    ui.checkbox(&mut dialog.bake_light, "Shade the vertex colours from above");
                    ui.end_row();

                    ui.label("Brightness");
                    ui.add(egui::Slider::new(&mut dialog.brightness, 0.25..=4.0));
                    ui.end_row();

                    ui.label("Sky");
                    egui::ComboBox::from_id_source("new_map_sky")
                        .selected_text(if dialog.sky_preset.is_empty() { "The scene's own" } else { dialog.sky_preset.as_str() })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut dialog.sky_preset, String::new(), "The scene's own");
                            for (name, _, _) in sky::PRESETS {
                                if ui.selectable_value(&mut dialog.sky_preset, name.to_string(), name).clicked() {
                                    // a preset's colours and clouds, the rest stays as it was set
                                    if let Ok(preset) = SkyOptions::preset(name) {
                                        dialog.sky = SkyOptions {
                                            zenith: preset.zenith,
                                            horizon: preset.horizon,
                                            ground: preset.ground,
                                            cloud_colour: preset.cloud_colour,
                                            clouds: preset.clouds,
                                            ..dialog.sky.clone()
                                        };
                                    }
                                    dialog.sky_picture = None;
                                }
                            }
                        });
                    ui.end_row();
                });
                if !dialog.sky_preset.is_empty() {
                    Self::show_sky_settings(ui, dialog);
                }
                ui.add_space(4.0);
                ui.label("Meshes named collision..., col_... or ucx_... are collided with and not drawn. Without any, the drawn triangles are collided with (not those of a material named ...nocollide...). A node named spawn... is where the player starts. Nodes named RoomXX are rooms and a quad named Portal_XX_YY in an opening joins two: the game then only draws the rooms that are seen. A quad named vault..., vault_long... or climb... on an obstacle's top is what the player gets over or onto, an upright one named ladder... is climbed.");
                ui.add_space(4.0);
                create = ui
                    .add_enabled(
                        !dialog.gltf.is_empty() && !dialog.folder.is_empty(),
                        egui::Button::new("Create"),
                    )
                    .clicked();
                if !dialog.status.is_empty() {
                    ui.label(&dialog.status);
                }
            });

        if create {
            use eurochef_shared::ge::{gltf_import::ImportOptions, project};
            let options = project::NewMapOptions {
                name: dialog.name.clone(),
                id: dialog.id,
                spawn: None,
                import: ImportOptions {
                    scale: dialog.scale,
                    bake_light: dialog.bake_light,
                    brightness: dialog.brightness,
                    ..Default::default()
                },
                sky: (!dialog.sky_preset.is_empty()).then(|| dialog.sky.clone()),
                split: true,
            };
            let made = project::new_map_from_gltf(&dialog.gltf, &options).and_then(|mut map| {
                let saved = map.save(&dialog.folder)?;
                Ok((saved, map))
            });
            match made {
                Ok((saved, map)) => {
                    // the level itself is only triggers: the file with the geometry is the one to look at
                    let path = saved.iter().find(|f| f.mode.is_none() && f.hash != map.level_hash).unwrap_or(&saved[0]).path.clone();
                    dialog.status = format!(
                        "Wrote {} and {} more file(s) to {} (level {:08X}): {} triangles, {} collided with, {} textures, {} rooms, {} portals. Start the game with GE_LEVEL={:08X}.",
                        saved[0].path.file_name().unwrap_or_default().to_string_lossy(),
                        saved.len() - 1,
                        dialog.folder,
                        map.level_hash,
                        map.stats.drawn_triangles,
                        map.stats.collision_triangles,
                        map.stats.textures,
                        map.stats.zones,
                        map.stats.portals,
                        map.level_hash
                    );
                    if !map.modes.is_empty() {
                        let modes: Vec<&str> = map.modes.iter().map(|m| m.folder()).collect();
                        dialog.status += &format!(" Gamemodes: {}.", modes.join(", "));
                    }
                    for (mode, why) in &map.left_out {
                        dialog.status += &format!(" No {}: {why}.", mode.folder());
                    }
                    for warning in &map.stats.warnings {
                        dialog.status += &format!(" Warning: {warning}.");
                    }
                    if let Err(e) = self.load_file_with_path(&path) {
                        self.state = AppState::Error(e);
                    }
                }
                Err(e) => dialog.status = format!("Not made: {e:#}"),
            }
        }
        if !open {
            self.new_map = None;
        }
    }

    pub fn screenshot_request(&mut self, path: String, panel: &str, after_frames: u32) {
        self.screenshot = Some((path, panel.to_lowercase(), after_frames.max(2)));
    }

    pub fn view_request(&mut self, camera: [f32; 5], selected: Option<usize>) {
        self.view = Some((camera, selected));
    }

    /// For pictures: the map view without the windows over it
    pub fn no_windows_request(&mut self) {
        self.no_windows = true;
    }

    /// For pictures: the new map dialog, filled in
    pub fn new_map_request(&mut self, gltf: String) {
        let mut dialog = NewMapDialog::default();
        dialog.name = std::path::Path::new(&gltf).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        dialog.folder = "mods".to_string();
        dialog.gltf = gltf;
        dialog.sky_preset = "dusk".to_string();
        dialog.sky = SkyOptions { clouds: 0.5, ..SkyOptions::preset("dusk").expect("the dusk preset") };
        self.new_map = Some(dialog);
    }

    /// Counts down to the requested screenshot, saves it when it arrives and closes the window
    fn update_screenshot(&mut self, ctx: &egui::Context) {
        let Some((path, panel, frames_left)) = self.screenshot.as_mut() else {
            return;
        };

        if self.pending_file.is_none() && self.fileinfo.is_some() {
            let wanted = match panel.as_str() {
                "info" => Some(Panel::FileInfo),
                "text" if self.spreadsheetlist.is_some() => Some(Panel::Spreadsheets),
                "textures" if self.textures.is_some() => Some(Panel::Textures),
                "entities" if self.entities.is_some() => Some(Panel::Entities),
                "scripts" if self.scripts.is_some() => Some(Panel::Scripts),
                "maps" if self.maps.is_some() => Some(Panel::Maps),
                _ => None,
            };
            if let Some(wanted) = wanted {
                self.current_panel = wanted;
            }
            if let (true, Some(maps)) = (self.no_windows, self.maps.as_mut()) {
                maps.hide_windows();
            }
            if let (Some((c, selected)), Some(maps)) = (self.view, self.maps.as_mut()) {
                maps.set_view(glam::Vec3::new(c[0], c[1], c[2]), c[3], c[4], selected);
            }
        }

        if *frames_left > 1 {
            *frames_left -= 1;
            ctx.request_repaint();
        } else if *frames_left == 1 {
            *frames_left = 0;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
            ctx.request_repaint();
        }

        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            let res = std::fs::File::create(&*path).map(|f| {
                let mut encoder =
                    png::Encoder::new(std::io::BufWriter::new(f), image.width() as u32, image.height() as u32);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                encoder
                    .write_header()
                    .and_then(|mut w| w.write_image_data(bytemuck::cast_slice(&image.pixels)))
            });
            match res {
                Ok(Ok(())) => info!("Saved screenshot to {path}"),
                Ok(Err(e)) => error!("Failed to write screenshot {path}: {e}"),
                Err(e) => error!("Failed to create screenshot {path}: {e}"),
            }
            self.screenshot = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    // TODO: Error handling
    pub fn load_file_with_path<P: AsRef<std::path::Path>>(
        &mut self,
        path: P,
    ) -> anyhow::Result<()> {
        let platform = Platform::from_path(&path);

        if let Some(dissected_path) = DissectedFilelistPath::dissect(&path) {
            self.game = dissected_path.game.clone();

            self.hashcodes = Arc::new(eurochef_shared::filesystem::load_hashcodes(
                &dissected_path,
                true,
            ));

            // Index the folder and load it into the path cache
            info!(
                "Indexing game folder {}",
                dissected_path.dir_relative().to_string_lossy()
            );
            self.path_cache.clear();

            for entry in glob::glob(&format!(
                "{}/*.edb",
                dissected_path.dir_absolute().to_string_lossy()
            ))? {
                match entry {
                    Ok(path) => {
                        let file = File::open(&path)?;
                        let mut reader = BufReader::new(file);
                        let endian = if reader.read_ne::<u8>()? == 0x47 {
                            Endian::Big
                        } else {
                            Endian::Little
                        };
                        reader.seek(std::io::SeekFrom::Start(4))?;
                        let hashcode: Hashcode = reader.read_type(endian)?;
                        self.path_cache
                            .insert(hashcode, path.to_string_lossy().to_string());
                    }
                    Err(e) => println!("{:?}", e),
                }
            }

            info!("Indexed {} EDBs", self.path_cache.len());
        }

        self.pending_path = path.as_ref().to_string_lossy().to_string();
        let mut f = File::open(path)?;
        let mut data = vec![];
        f.read_to_end(&mut data)?;
        self.pending_file = Some((data, platform));

        Ok(())
    }

    pub fn load_into_render_store(
        &mut self,
        references: &[Hashcode],
        file_map: &mut IntMap<Hashcode, EdbFile>,
        file_ref: Hashcode,
        platform: Platform,
    ) -> anyhow::Result<()> {
        let edb = &mut (if let Some(path) = self.path_cache.get(&file_ref) {
            match file_map.entry(file_ref) {
                hash_map::Entry::Occupied(e) => e.into_mut(),
                hash_map::Entry::Vacant(a) => {
                    let file = File::open(path)?;
                    let reader = BufReader::new(file);

                    a.insert(EdbFile::new(Box::new(reader), platform)?)
                }
            }
        } else {
            return Ok(());
        });

        let header = edb.header.clone();

        let mut rs_lock = self.render_store.write();
        let scripts = UXGeoScript::read_hashcodes(edb, references)?;
        for s in &scripts {
            rs_lock.insert_script(header.hashcode, s.clone());
        }

        // Entities should come after scripts, since we need all references to resolve first
        // Also include the requested references
        let interal_references_filtered: Vec<Hashcode> = edb
            .internal_references
            .iter()
            .filter(|v| !rs_lock.is_object_loaded(header.hashcode, **v))
            .copied()
            .collect();
        let internal_refs = [references, &interal_references_filtered].concat();
        let (entities, _, _) = entities::read_from_file(edb, Some(&internal_refs))?;
        for (i, e) in entities.into_iter() {
            let mut r = EntityRenderer::new(header.hashcode, edb.platform);
            if let Ok((_, m)) = &e.data {
                unsafe {
                    r.load_mesh(&self.gl, m);
                }
            }
            rs_lock.insert_entity(header.hashcode, e.hashcode, i, r);
        }

        // Textures should come last, since textures refer to nothing (aside from a few external references)
        let internal_refs = edb.internal_references.clone();
        let textures = UXGeoTexture::read_hashcodes(edb, &internal_refs);
        for (i, t) in entities::EntityListPanel::load_textures(&self.gl, &textures) {
            rs_lock.insert_texture(header.hashcode, t.hashcode, i, t);
        }

        drop(rs_lock);
        let external_references = edb.external_references.clone();
        self.resolve_references(platform, &external_references, file_map)?;

        Ok(())
    }

    pub fn resolve_references(
        &mut self,
        platform: Platform,
        references: &[(Hashcode, Hashcode)],
        file_map: &mut IntMap<Hashcode, EdbFile>,
    ) -> anyhow::Result<()> {
        let mut grouped_refs: Vec<(Hashcode, Vec<Hashcode>)> = vec![];
        for (rf, ro) in references {
            let group = if let Some(f) = grouped_refs.iter_mut().find(|(f, _)| f == rf) {
                f
            } else {
                grouped_refs.push((*rf, vec![]));
                grouped_refs.last_mut().unwrap()
            };

            if !group.1.contains(ro) && !self.render_store.read().is_object_loaded(*rf, *ro) {
                group.1.push(*ro)
            }
        }

        grouped_refs.retain(|v| !v.1.is_empty());

        for (ref_file, ref_objects) in grouped_refs {
            self.load_into_render_store(&ref_objects, file_map, ref_file, platform)?;
        }

        Ok(())
    }

    pub fn load_file<R: Read + Seek + 'static>(
        &mut self,
        platform: Platform,
        reader: Box<R>,
        ctx: &egui::Context,
    ) -> anyhow::Result<()> {
        if platform == Platform::Ps2 {
            self.ps2_warning = true;
        }

        self.render_store.write().purge(true);
        let mut edb = EdbFile::new(reader, platform)?;
        let header = edb.header.clone();

        self.current_panel = Panel::FileInfo;
        self.spreadsheetlist = None;
        self.fileinfo = None;
        self.textures = None;
        self.maps = None;
        self.scripts = None;

        self.fileinfo = Some(fileinfo::FileInfoPanel::new(edb.header.clone()));

        // a file on its own, outside the game's folders: version 263 on the Wii is taken for
        // GoldenEye 007, whose folder is bondx
        if DissectedFilelistPath::dissect(&self.pending_path).is_none() {
            self.game = if header.version == 263 && platform == Platform::Wii {
                "bondx".to_string()
            } else {
                String::new()
            };
        }

        let spreadsheets = UXGeoSpreadsheet::read_all(&mut edb)?;
        if !spreadsheets.is_empty() {
            self.spreadsheetlist = Some(spreadsheet::TextItemList::new(spreadsheets.clone()));
        }

        if [
            Platform::Xbox,
            Platform::Xbox360,
            Platform::Pc,
            Platform::Ps2,
            Platform::GameCube,
            Platform::Wii,
        ]
        .contains(&platform)
        {
            let (entities, skins, ref_entities) = entities::read_from_file(&mut edb, None)?;

            for (i, e) in entities.iter() {
                if e.hashcode.is_local() {
                    debug_assert_eq!(e.hashcode.index(), *i as u32);
                }
            }

            let mut rs_lock = self.render_store.write();
            let scripts = UXGeoScript::read_all(&mut edb)?;
            for s in &scripts {
                rs_lock.insert_script(header.hashcode, s.clone());
            }

            if !scripts.is_empty() {
                self.scripts = Some(scripts::ScriptListPanel::new(
                    header.hashcode,
                    &self.gl,
                    scripts,
                    self.render_store.clone(),
                    self.hashcodes.clone(),
                ));
            }

            for (i, e) in entities.iter() {
                let mut r = EntityRenderer::new(header.hashcode, platform);
                if let Ok((_, m)) = &e.data {
                    unsafe {
                        r.load_mesh(&self.gl, m);
                    }
                }
                rs_lock.insert_entity(header.hashcode, e.hashcode, *i, r);
            }

            if entities.len() + skins.len() + ref_entities.len() > 0 {
                if self.fileinfo.as_ref().unwrap().header.map_list.len() > 0 {
                    let map = maps::read_from_file(&mut edb);

                    self.maps = Some(maps::MapViewerPanel::new(
                        header.hashcode,
                        self.gl.clone(),
                        map,
                        ref_entities.clone(),
                        self.render_store.clone(),
                        platform,
                        self.hashcodes.clone(),
                        &self.game,
                    ));
                }

                self.entities = Some(entities::EntityListPanel::new(
                    header.hashcode,
                    self.render_store.clone(),
                    ctx,
                    self.gl.clone(),
                    entities.into_iter().map(|(_, ires)| ires).collect(),
                    skins,
                    ref_entities,
                    platform,
                ));
            }
        } else {
            self.entities = None;
        }

        let textures = UXGeoTexture::read_all(&mut edb);
        {
            let mut rs_lock = self.render_store.write();
            for (i, t) in entities::EntityListPanel::load_textures(&self.gl, &textures).into_iter()
            {
                rs_lock.insert_texture(header.hashcode, t.hashcode, i, t);
            }
        }

        if textures.len() == 1 && textures[0].1.hashcode == 0x06000000 {
            self.textures = None;
        } else {
            self.textures = Some(textures::TextureList::new(
                ctx,
                textures.into_iter().map(|(_, t)| t).collect(),
            ));
        }

        edb.external_references.sort_by(|(a, _), (b, _)| a.cmp(b));
        self.fileinfo.as_mut().unwrap().external_references = edb.external_references.clone();

        let start = Instant::now();
        let mut file_map: IntMap<Hashcode, EdbFile> = Default::default();
        self.resolve_references(platform, &edb.external_references, &mut file_map)?;
        info!(
            "Resolving references took {}s",
            start.elapsed().as_secs_f32()
        );

        self.state = AppState::Ready;

        Ok(())
    }
}

impl eframe::App for EurochefApp {
    /// Called by the frame work to save state before shutdown.
    fn save(&mut self, _storage: &mut dyn eframe::Storage) {}

    /// Called each time the UI needs repainting, which may be many times per second.
    /// Put your widgets into a `SidePanel`, `TopPanel`, `CentralPanel`, `Window` or `Area`.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        puffin::set_scopes_on(self.show_profiler);
        puffin::GlobalProfiler::lock().new_frame();

        egui::Window::new("Profiler")
            .open(&mut self.show_profiler)
            .show(ctx, |ui| {
                puffin_egui::profiler_ui(ui);
            });

        if let Some((data, load_path)) = self.load_input.take() {
            let platform = Platform::from_path(&load_path);
            self.pending_path = load_path;
            self.pending_file = Some((data, platform));
        }

        if let Some((data, platform)) = self.pending_file.as_ref() {
            if let Some(platform) = platform {
                let cur = Cursor::new(data.clone()); // FIXME: Cloning the data hurts my soul
                let source = (self.pending_path.clone(), Arc::new(data.clone()));
                match self.load_file(*platform, Box::new(cur), ctx) {
                    Ok(_) => {
                        if let Some(maps) = self.maps.as_mut() {
                            maps.set_source(source.0.clone(), source.1.clone());
                        }
                        self.current_source = Some(source);
                    }
                    Err(e) => {
                        self.state = AppState::Error(e);
                    }
                }
                self.pending_file = None;

                if let Some(target) = self.save_triggers_to.take() {
                    match self.maps.as_ref().map(|m| m.save_triggers(std::path::Path::new(&target))) {
                        Some(Ok(size)) => info!("Wrote the triggers back: {target} ({size} bytes)"),
                        Some(Err(e)) => error!("Couldn't write {target}: {e:#}"),
                        None => error!("Couldn't write {target}: the file has no map"),
                    }
                }
            } else {
                self.state = AppState::SelectPlatform;
            }
        }

        self.update_screenshot(ctx);
        self.show_new_map(ctx);

        let Self {
            state,
            current_panel,
            spreadsheetlist,
            fileinfo,
            textures,
            load_input,
            entities,
            scripts,
            maps,
            selected_platform,
            ..
        } = self;

        let load_clone = load_input.clone();

        // swy: queue a load for the first drag-and-dropped file we encounter here
        ctx.input(|i| {
            if !i.raw.dropped_files.is_empty() {
                for file in &i.raw.dropped_files {
                    let info = if let Some(path) = &file.path {
                        path.display().to_string()
                    } else if !file.name.is_empty() {
                        file.name.clone()
                    } else {
                        "???".to_owned()
                    };

                    info!("Dragged a into the main window: '{info}'");

                    // swy: put the path and its data inside load_input, load_clone is like a pointer
                    match File::open(&info) {
                        Err(why) => warn!("Couldn't read '{info}', skipping: {why}"),
                        Ok(mut f) => {
                            let mut data = vec![];
                            f.read_to_end(&mut data).unwrap();

                            load_clone.store(Some((data, info)));

                            // swy: skip the rest, for the time being, we only care about the first one
                            break;
                        }
                    }
                }
            }
        });

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open").clicked() {
                        // TODO(cohae): drag and drop loading
                        #[cfg(target_arch = "wasm32")]
                        {
                            wasm_bindgen_futures::spawn_local(async move {
                                let future = rfd::AsyncFileDialog::new()
                                    .add_filter("Eurocom DB", &["edb"])
                                    .pick_file();
                                if let Some(file) = future.await {
                                    let data = file.read().await;
                                    info!("{}", file.file_name());
                                    load_clone.store(Some((data, file.file_name())));
                                } else {
                                }
                            });
                        }

                        #[cfg(not(target_arch = "wasm32"))]
                        std::thread::spawn(move || {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("EngineX Database", &["edb"])
                                .pick_file()
                            {
                                let mut f = File::open(&path).unwrap();
                                let mut data = vec![];
                                f.read_to_end(&mut data).unwrap();

                                load_clone.store(Some((data, path.to_string_lossy().to_string())));
                            } else {
                                load_clone.store(None);
                            }
                        });

                        ui.close_menu()
                    }
                });

                if ui.button("New GoldenEye 007 map").clicked() {
                    self.new_map.get_or_insert_with(NewMapDialog::default);
                }

                if ui.button("Profiler").clicked() {
                    self.show_profiler = true;
                }

                if ui.button("About").clicked() {
                    self.about_window = true;
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let style: egui::Style = (*ui.ctx().style()).clone();
                    let new_visuals = style.visuals.light_dark_small_toggle_button(ui);
                    if let Some(visuals) = new_visuals {
                        ui.ctx().set_visuals(visuals);
                    }
                });
            });
        });

        // Run the app at refresh rate on the texture panel (for animated textures)
        match current_panel {
            Panel::Entities | Panel::Textures | Panel::Maps | Panel::Scripts => {
                ctx.request_repaint()
            }
            _ => {
                ctx.request_repaint_after(std::time::Duration::from_secs_f32(1.));
            }
        }

        let screen_rect = ctx.screen_rect();
        let max_height = 320.0.at_most(screen_rect.height());

        if self.about_window {
            egui::Window::new("About")
                .pivot(egui::Align2::CENTER_TOP)
                .fixed_pos(screen_rect.center() - 0.5 * max_height * egui::Vec2::Y)
                .frame(
                    egui::Frame::window(&ctx.style()).inner_margin(egui::Margin {
                        left: 16.0,
                        right: 16.0,
                        ..Default::default()
                    }),
                )
                .resizable(false)
                .collapsible(false)
                .open(&mut self.about_window)
                .show(ctx, |ui| {
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.heading(egui::RichText::new("Eurochef").color(egui::Color32::WHITE));
                        ui.heading(format!(
                            "- {} ({})",
                            env!("CARGO_PKG_VERSION"),
                            &env!("GIT_HASH")[..7]
                        ));
                    });
                    ui.add_space(8.0);

                    ui.label(format!("Compiler: {}", env!("RUSTC_VERSION")));
                    ui.label(format!("Build date: {}", env!("BUILD_DATE")));

                    ui.add_space(12.0);
                });
        }

        // TODO(cohae): More generic dialog (use for loading and error)
        if self.ps2_warning {
            egui::Window::new("PS2 Support")
            .pivot(egui::Align2::CENTER_TOP)
            .fixed_pos(screen_rect.center() - 0.5 * max_height * egui::Vec2::Y)
            .frame(egui::Frame::window(&ctx.style()).inner_margin(16.))
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (irect, _) =
                        ui.allocate_exact_size([54., 54.].into(), egui::Sense::hover());
                    ui.painter().text(
                        irect.center() + [0., 8.].into(),
                        egui::Align2::CENTER_CENTER,
                        font_awesome::EXCLAMATION_TRIANGLE,
                        egui::FontId::proportional(48.),
                        Color32::from_rgb(249, 239, 40),
                    );

                    ui.label("PS2 support is currently highly experimental.\nTextures work, but most entities will not draw properly.");
                });
                if ui.button("I understand").clicked() {
                    self.ps2_warning = false;
                }
            });
        }

        match state {
            AppState::Ready => {}
            AppState::Loading(s) => {
                egui::Window::new("Loading")
                    .title_bar(false)
                    .pivot(egui::Align2::CENTER_TOP)
                    .fixed_pos(screen_rect.center() - 0.5 * max_height * egui::Vec2::Y)
                    .frame(egui::Frame::window(&ctx.style()).inner_margin(16.))
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(s.as_str());
                        });
                    });
            }
            AppState::SelectPlatform => {
                egui::Window::new("Select platform")
                    .title_bar(false)
                    .pivot(egui::Align2::CENTER_TOP)
                    .fixed_pos(screen_rect.center() - 0.5 * max_height * egui::Vec2::Y)
                    .frame(egui::Frame::window(&ctx.style()).inner_margin(16.))
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.heading("Please select the platform for this file");
                        ui.separator();
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.strong("Platform: ");
                            egui::ComboBox::from_label("")
                                .selected_text(selected_platform.to_string())
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        selected_platform,
                                        Platform::GameCube,
                                        "GameCube",
                                    );
                                    ui.selectable_value(selected_platform, Platform::Pc, "PC");
                                    ui.selectable_value(
                                        selected_platform,
                                        Platform::Ps2,
                                        "PlayStation 2",
                                    );
                                    ui.selectable_value(
                                        selected_platform,
                                        Platform::Ps3,
                                        "PlayStation 3",
                                    );
                                    ui.selectable_value(
                                        selected_platform,
                                        Platform::ThreeDS,
                                        "3DS",
                                    );
                                    ui.selectable_value(selected_platform, Platform::Wii, "Wii");
                                    ui.selectable_value(selected_platform, Platform::WiiU, "Wii U");
                                    ui.selectable_value(selected_platform, Platform::Xbox, "Xbox");
                                    ui.selectable_value(
                                        selected_platform,
                                        Platform::Xbox360,
                                        "Xbox 360",
                                    );
                                });
                        });

                        ui.horizontal(|ui| {
                            if ui.button("Load").clicked() {
                                if let Some((_, platform)) = self.pending_file.as_mut() {
                                    *platform = Some(*selected_platform);
                                }
                                self.state = AppState::Loading("Loading file".to_string());
                            }

                            if ui.button("Cancel").clicked() {
                                self.pending_file = None;
                                self.state = AppState::Ready;
                            }
                        });
                    });
            }
            AppState::Error(e) => {
                let mut open = true;
                egui::Window::new("Error")
                    .pivot(egui::Align2::CENTER_TOP)
                    .fixed_pos(screen_rect.center() - 0.5 * max_height * egui::Vec2::Y)
                    // .frame(egui::Frame::window(&ctx.style()).inner_margin(16.))
                    .resizable(false)
                    .collapsible(false)
                    .open(&mut open)
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            let (irect, _) =
                                ui.allocate_exact_size([48., 48.].into(), egui::Sense::hover());
                            ui.painter().text(
                                irect.center() + [0., 8.].into(),
                                egui::Align2::CENTER_CENTER,
                                '\u{f00d}',
                                egui::FontId::proportional(48.),
                                Color32::from_rgb(250, 40, 40),
                            );

                            ui.label(remove_stacktrace(&format!("{e:?}")));
                        });

                        if !e.backtrace().to_string().starts_with("disabled backtrace") {
                            ui.add_space(4.);
                            ui.collapsing("Backtrace", |ui| {
                                egui::ScrollArea::vertical()
                                    .show(ui, |ui| ui.label(e.backtrace().to_string()));
                            });
                        }
                    });

                if !open {
                    *state = AppState::Ready;
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            if fileinfo.is_none() {
                ui.heading("No file loaded");
                return;
            }

            ui.horizontal(|ui| {
                if fileinfo.is_some() {
                    ui.selectable_value(current_panel, Panel::FileInfo, "File info");
                }

                if spreadsheetlist.is_some() {
                    ui.selectable_value(current_panel, Panel::Spreadsheets, "Text");
                }

                if textures.is_some() {
                    ui.selectable_value(current_panel, Panel::Textures, "Textures");
                }

                if entities.is_some() {
                    ui.selectable_value(current_panel, Panel::Entities, "Entities");
                }

                if scripts.is_some() {
                    ui.selectable_value(current_panel, Panel::Scripts, "Scripts");
                }

                if maps.is_some() {
                    ui.selectable_value(current_panel, Panel::Maps, "Maps");
                }
            });
            ui.separator();

            match current_panel {
                Panel::FileInfo => fileinfo
                    .as_mut()
                    .map(|s| s.show(ui, &self.hashcodes, &self.render_store.read())),
                Panel::Textures => textures.as_mut().map(|s| s.show(ui)),
                Panel::Entities => entities.as_mut().map(|s| s.show(ctx, ui)),
                Panel::Spreadsheets => spreadsheetlist.as_mut().map(|s| s.show(ui)),
                Panel::Maps => {
                    if let Some(Err(e)) = maps.as_mut().map(|s| s.show(ctx, ui)) {
                        self.state = AppState::Error(e);
                    };
                    Some(())
                }
                Panel::Scripts => scripts.as_mut().map(|s| s.show(ui)),
            };
        });

        // TODO(cohae): Should be implemented in `TextureList::show`
        match current_panel {
            Panel::Textures => textures.as_mut().map(|s| s.show_enlarged_window(ctx)),
            _ => None,
        };
    }
}

fn remove_stacktrace(s: &str) -> &str {
    if let Some(v) = s.to_lowercase().find("stack backtrace:") {
        s[..v].trim()
    } else {
        s
    }
}
