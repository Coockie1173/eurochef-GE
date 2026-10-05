use std::{io::Seek, sync::Arc};

use anyhow::Context;

use egui::mutex::{Mutex, RwLock};
use eurochef_edb::{
    binrw::BinReaderExt,
    edb::EdbFile,
    entity::{EXGeoEntity, EXGeoMapZoneEntity},
    map::{EXGeoBaseDatum, EXGeoMap, EXGeoMapZone, EXGeoPlacement, EXGeoTriggerEngineOptions},
    versions::Platform,
    Hashcode,
};
use eurochef_shared::IdentifiableResult;
use glam::Vec3;
use nohash_hasher::IntMap;

use crate::{
    entities::ProcessedEntityMesh,
    map_frame::MapFrame,
    render::{entity::EntityRenderer, viewer::CameraType, RenderStore},
};

pub struct MapViewerPanel {
    maps: Vec<ProcessedMap>,

    /// The file the maps were read from, for writing changed triggers back into it
    source: Option<(String, Arc<Vec<u8>>)>,
    editor: TriggerEditor,

    // TODO(cohae): Replace so we can do funky stuff
    frame: MapFrame,
}

#[derive(Clone)]
pub struct ProcessedMap {
    pub hashcode: u32,
    pub mapzone_entities: Vec<EXGeoMapZoneEntity>,
    pub zones: Vec<EXGeoMapZone>,
    pub skies: Vec<Hashcode>,
    pub placements: Vec<EXGeoPlacement>,
    pub triggers: Vec<ProcessedTrigger>,
    pub trigger_collisions: Vec<EXGeoBaseDatum>,
}

#[derive(Clone)]
pub struct ProcessedTrigger {
    pub link_ref: i32,

    pub ttype: u32,
    pub tsubtype: Option<u32>,
    /// The subtype as the file has it
    pub raw_subtype: u32,

    pub debug: u16,
    pub game_flags: u32,
    pub trig_flags: u32,
    pub position: Vec3,
    pub rotation: Vec3,
    pub scale: Vec3,

    pub data: Vec<Option<u32>>,
    pub links: Vec<i32>,
    pub engine_options: EXGeoTriggerEngineOptions,

    /// Every trigger that links to this one
    pub incoming_links: Vec<i32>,
}

impl MapViewerPanel {
    pub fn new(
        file: Hashcode,
        gl: Arc<glow::Context>,
        maps: Vec<ProcessedMap>,
        ref_entities: Vec<IdentifiableResult<(EXGeoEntity, ProcessedEntityMesh)>>,
        render_store: Arc<RwLock<RenderStore>>,
        platform: Platform,
        hashcodes: Arc<IntMap<u32, String>>,
        game: &str,
    ) -> Self {
        MapViewerPanel {
            frame: {
                let ef = MapFrame::new(
                    file,
                    Self::load_map_meshes(file, &gl, &maps, &ref_entities, platform),
                    gl,
                    render_store,
                    hashcodes,
                    game,
                );

                {
                    let mut e = ef.viewer.lock();
                    e.selected_camera = CameraType::Fly;
                    e.show_grid = false;
                }

                ef
            },
            maps,
            source: None,
            editor: TriggerEditor::default(),
        }
    }

    pub fn set_source(&mut self, path: String, data: Arc<Vec<u8>>) {
        self.source = Some((path, data));
    }

    fn load_map_meshes(
        file: Hashcode,
        gl: &glow::Context,
        maps: &[ProcessedMap],
        ref_entities: &[IdentifiableResult<(EXGeoEntity, ProcessedEntityMesh)>],
        platform: Platform,
    ) -> Vec<(u32, Arc<Mutex<EntityRenderer>>)> {
        let mut ref_renderers = vec![];

        // FIXME(cohae): Map picking is a bit dirty at the moment
        for map in maps.iter() {
            for v in &map.mapzone_entities {
                if let Some(Ok((_, e))) = &ref_entities
                    .iter()
                    .find(|ir| ir.hashcode == v.entity_refptr)
                    .map(|v| v.data.as_ref())
                {
                    let r = Arc::new(Mutex::new(EntityRenderer::new(file, platform)));
                    unsafe {
                        r.lock().load_mesh(gl, e);
                    }
                    ref_renderers.push((map.hashcode, r));
                } else {
                    error!(
                        "Couldn't find ref entity #{} for mapzone entity!",
                        v.entity_refptr
                    );
                }
            }
        }

        ref_renderers
    }

    pub fn show(&mut self, context: &egui::Context, ui: &mut egui::Ui) -> anyhow::Result<()> {
        self.show_editor(context);
        self.frame.show(ui, context, &self.maps)
    }

    fn relink(map: &mut ProcessedMap) {
        for i in 0..map.triggers.len() {
            map.triggers[i].incoming_links = (0..map.triggers.len())
                .filter(|e| *e != i && map.triggers[*e].links.iter().any(|v| *v == i as i32))
                .map(|e| e as i32)
                .collect();
        }
    }

    /// The window that moves, adds and removes triggers (the game's entities) and writes the
    /// file again
    fn show_editor(&mut self, ctx: &egui::Context) {
        let selected_map = self.frame.selected_map;
        let Some(map) = self.maps.get_mut(selected_map) else {
            return;
        };
        let mut selected = self.frame.selected_trigger.filter(|i| *i < map.triggers.len());
        let camera = self.frame.viewer.lock().camera_mut().position();
        let editor = &mut self.editor;
        let trigger_info = self.frame.trigger_info.clone();
        let type_name = |ttype: u32| match trigger_info.triggers.get(&ttype) {
            Some(def) => format!("{} (0x{:x})", def.name, ttype),
            None => format!("0x{:x}", ttype),
        };
        let mut save_clicked = false;

        egui::Window::new("Entities")
            .default_pos([ctx.screen_rect().right() - 340.0, 110.0])
            .default_width(310.0)
            .scroll([false, true])
            .show(ctx, |ui| {
                ui.label(format!("{} triggers in this map", map.triggers.len()));
                ui.horizontal(|ui| {
                    ui.label("Type");
                    ui.add(egui::DragValue::new(&mut editor.new_type).hexadecimal(2, false, false));
                    if ui.button("Add at the camera").clicked() {
                        let defaults = eurochef_shared::ge::triggers::GeTrigger::with_defaults(
                            editor.new_type,
                            camera.into(),
                        );
                        map.triggers.push(ProcessedTrigger {
                            link_ref: -1,
                            ttype: editor.new_type,
                            tsubtype: None,
                            raw_subtype: 0,
                            debug: 0,
                            game_flags: 0,
                            trig_flags: 0,
                            position: camera,
                            rotation: Vec3::new(0.0, -0.0, -0.0),
                            scale: Vec3::ONE,
                            data: defaults.data.to_vec(),
                            links: vec![-1; 8],
                            engine_options: Default::default(),
                            incoming_links: vec![],
                        });
                        selected = Some(map.triggers.len() - 1);
                        editor.changed = true;
                    }
                });
                ui.separator();

                match selected {
                    None => {
                        ui.label("Click a trigger in the map to change it.");
                    }
                    Some(index) => {
                        let mut duplicate = false;
                        let mut delete = false;
                        {
                            let trigger = &mut map.triggers[index];
                            ui.strong(format!("Trigger {index}: {}", type_name(trigger.ttype)));
                            egui::Grid::new("trigger_edit").num_columns(2).show(ui, |ui| {
                                for (label, vec, speed) in [
                                    ("Position", &mut trigger.position, 0.05),
                                    ("Rotation", &mut trigger.rotation, 0.01),
                                    ("Scale", &mut trigger.scale, 0.01),
                                ] {
                                    ui.label(label);
                                    ui.horizontal(|ui| {
                                        for axis in [&mut vec.x, &mut vec.y, &mut vec.z] {
                                            editor.changed |= ui
                                                .add(egui::DragValue::new(axis).speed(speed).max_decimals(3))
                                                .changed();
                                        }
                                    });
                                    ui.end_row();
                                }
                                ui.label("Flags");
                                editor.changed |= ui
                                    .add(egui::DragValue::new(&mut trigger.game_flags).hexadecimal(1, false, false))
                                    .changed();
                                ui.end_row();
                            });

                            ui.collapsing("Values", |ui| {
                                egui::Grid::new("trigger_values").num_columns(3).show(ui, |ui| {
                                    for slot in 0..trigger.data.len().min(16) {
                                        let name = trigger_info
                                            .triggers
                                            .get(&trigger.ttype)
                                            .and_then(|d| d.values.get(&(slot as u32)))
                                            .and_then(|v| v.name.clone())
                                            .unwrap_or_else(|| format!("Value {slot}"));
                                        let mut used = trigger.data[slot].is_some();
                                        if ui.checkbox(&mut used, name).changed() {
                                            trigger.data[slot] = used.then_some(0);
                                            editor.changed = true;
                                        }
                                        if let Some(value) = trigger.data[slot].as_mut() {
                                            editor.changed |= ui
                                                .add(egui::DragValue::new(value).hexadecimal(1, false, false))
                                                .changed();
                                            let mut float = f32::from_bits(*value);
                                            if ui
                                                .add(egui::DragValue::new(&mut float).speed(0.05).max_decimals(3))
                                                .on_hover_text("The same value as a float")
                                                .changed()
                                            {
                                                *value = float.to_bits();
                                                editor.changed = true;
                                            }
                                        }
                                        ui.end_row();
                                    }
                                });
                            });

                            ui.horizontal(|ui| {
                                duplicate = ui.button("Duplicate").clicked();
                                delete = ui.button("Delete").clicked();
                            });
                        }

                        if duplicate {
                            let mut copy = map.triggers[index].clone();
                            copy.position += Vec3::new(1.0, 0.0, 0.0);
                            copy.incoming_links.clear();
                            map.triggers.push(copy);
                            selected = Some(map.triggers.len() - 1);
                            editor.changed = true;
                        }
                        if delete {
                            map.triggers.remove(index);
                            // links to it are dropped, links to the ones behind it follow them
                            for trigger in &mut map.triggers {
                                for link in &mut trigger.links {
                                    if *link == index as i32 {
                                        *link = -1;
                                    } else if *link > index as i32 {
                                        *link -= 1;
                                    }
                                }
                                if trigger.link_ref == index as i32 {
                                    trigger.link_ref = -1;
                                } else if trigger.link_ref > index as i32 {
                                    trigger.link_ref -= 1;
                                }
                            }
                            selected = None;
                            editor.changed = true;
                        }
                        if duplicate || delete {
                            Self::relink(map);
                        }
                    }
                }

                ui.separator();
                ui.horizontal(|ui| {
                    let can_save = self.source.is_some() && selected_map == 0;
                    save_clicked = ui
                        .add_enabled(can_save, egui::Button::new("Save EDB as..."))
                        .on_disabled_hover_text("Only the triggers of a file's first map can be written")
                        .clicked();
                    if editor.changed {
                        ui.label("changed");
                    }
                });
                if !editor.status.is_empty() {
                    ui.label(&editor.status);
                }
            });

        self.frame.selected_trigger = selected;

        if save_clicked {
            let (path, _) = self.source.as_ref().unwrap();
            let start = std::path::Path::new(path);
            let mut dialog = rfd::FileDialog::new().add_filter("EngineX Database", &["edb"]);
            if let Some(dir) = start.parent() {
                dialog = dialog.set_directory(dir);
            }
            if let Some(name) = start.file_name() {
                dialog = dialog.set_file_name(name.to_string_lossy());
            }
            if let Some(target) = dialog.save_file() {
                self.editor.status = match self.save_triggers(&target) {
                    Ok(size) => {
                        self.editor.changed = false;
                        format!("Wrote {} ({} bytes)", target.display(), size)
                    }
                    Err(e) => format!("Not written: {e:#}"),
                };
            }
        }
    }

    /// Writes the file with the first map's triggers as they are now
    pub fn save_triggers(&self, target: &std::path::Path) -> anyhow::Result<usize> {
        use eurochef_shared::ge::triggers::{read_triggers, write_triggers, GeTrigger};

        let (_, data) = self
            .source
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("the file's data isn't known"))?;
        let map = self.maps.first().ok_or_else(|| anyhow::anyhow!("no map"))?;
        let mut set = read_triggers(data)?;
        set.triggers = map
            .triggers
            .iter()
            .map(|t| {
                let mut out = GeTrigger::new(t.ttype, t.position.into());
                out.subtype = t.raw_subtype;
                out.debug = t.debug;
                out.game_flags = t.game_flags;
                out.rotation = t.rotation.into();
                out.scale = t.scale.into();
                out.link_ref = t.link_ref;
                for (slot, v) in t.data.iter().take(16).enumerate() {
                    out.data[slot] = *v;
                }
                // a link is there when the trigger's flags say so, or when it has been set
                for (slot, v) in t.links.iter().take(8).enumerate() {
                    if *v != -1 || t.trig_flags & (1 << (16 + slot)) != 0 {
                        out.links[slot] = Some(*v);
                    }
                }
                let e = &t.engine_options;
                out.engine = [
                    e.visual_object,
                    e.visual_object_file,
                    e.gamescript_index,
                    e.collision_index,
                    e.trigger_color.map(u32::from_be_bytes),
                    e._unk5,
                    e._unk6,
                    e._unk7,
                ];
                out
            })
            .collect();
        let out = write_triggers(data, &set)?;
        std::fs::write(target, &out).with_context(|| format!("couldn't write {}", target.display()))?;
        Ok(out.len())
    }
}

#[derive(Default)]
struct TriggerEditor {
    new_type: u32,
    changed: bool,
    status: String,
}

pub fn read_from_file(edb: &mut EdbFile) -> Vec<ProcessedMap> {
    let header = edb.header.clone();

    let mut maps = vec![];
    for m in header.map_list.iter() {
        edb.seek(std::io::SeekFrom::Start(m.address as u64))
            .unwrap();

        let xmap = edb
            .read_type_args::<EXGeoMap>(edb.endian, (header.version,))
            .context("Failed to read map")
            .unwrap();

        let mut map = ProcessedMap {
            hashcode: m.hashcode,
            mapzone_entities: vec![],
            placements: xmap.placements.data().clone(),
            triggers: vec![],
            trigger_collisions: xmap.trigger_header.trigger_collisions.0.clone(),
            skies: xmap.skies.iter().map(|s| s.hashcode).collect(),
            zones: vec![],
        };

        for z in &xmap.zones {
            let entity_offset = header.refpointer_list[z.entity_refptr as usize].address;
            edb.seek(std::io::SeekFrom::Start(entity_offset as u64))
                .context("Mapzone refptr pointer to a non-entity object!")
                .unwrap();

            let ent = edb
                .read_type_args::<EXGeoEntity>(edb.endian, (header.version, edb.platform))
                .unwrap();

            if let EXGeoEntity::MapZone(mapzone) = ent {
                map.mapzone_entities.push(mapzone);
            } else {
                error!("Refptr entity does not have a mapzone entity!");
                // Result::<()>::Err(anyhow::anyhow!(
                //     "Refptr entity does not have a mapzone entity!"
                // ))
                // .unwrap();
            }
        }

        map.zones = xmap.zones;

        for t in xmap.trigger_header.triggers.iter() {
            let trig = &t.trigger;
            let (ttype, tsubtype) = {
                let t = &xmap.trigger_header.trigger_types[trig.type_index as usize];

                (t.trig_type, t.trig_subtype)
            };

            let trigger = ProcessedTrigger {
                link_ref: t.link_ref,
                ttype,
                raw_subtype: tsubtype,
                tsubtype: if tsubtype != 0 && tsubtype != 0x42000001 {
                    Some(tsubtype)
                } else {
                    None
                },
                debug: trig.debug,
                game_flags: trig.game_flags,
                trig_flags: trig.trig_flags,
                position: trig.position.into(),
                rotation: trig.rotation.into(),
                scale: trig.scale.into(),
                engine_options: trig.engine_options.clone(),
                data: trig.data.to_vec(),
                links: trig.links.to_vec(),
                incoming_links: vec![],
            };

            map.triggers.push(trigger);
        }

        for i in 0..map.triggers.len() {
            for ei in 0..map.triggers.len() {
                if i == ei {
                    continue;
                }

                if map.triggers[ei].links.iter().any(|v| *v == i as i32) {
                    map.triggers[i].incoming_links.push(ei as i32);
                }
            }
        }

        maps.push(map);
    }

    maps
}
