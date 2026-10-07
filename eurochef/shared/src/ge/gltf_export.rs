//! Writes a map's triggers as a glTF scene: one empty node for each, where it stands and turned
//! as it is, to model a level around or to carry over into a new one.
//!
//! The names are the ones `gltf_import` reads: the player's spawn point (0x1C) is `spawn`, a
//! spawn point of a multiplayer game (0x35) `mp_spawn_NNN`, a team's `mp_spawn_teamT_NNN`. Every
//! other trigger is `trigger_NNN_TYPE` (its index in the map and its type's name, or the type in
//! hex), which the importer leaves alone. Each node's extras have all of the trigger's values.
//!
//! The game's levels are the mirror image of glTF, as in the importer: x is negated, and with it
//! a rotation's y and z.

use serde_json::{json, Map, Value};

use super::triggers::{GeTrigger, TRIGGER_MULTIPLAYER_SPAWN, TRIGGER_SPAWN};

/// The rotation the game makes of a trigger's angles (around z, then x, then y), as a
/// quaternion x, y, z, w
fn rotation_of(angles: [f32; 3]) -> [f32; 4] {
    let multiply = |a: [f32; 4], b: [f32; 4]| {
        [
            a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
            a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
            a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
            a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
        ]
    };
    let around = |axis: usize, angle: f32| {
        let mut q = [0.0, 0.0, 0.0, (angle * 0.5).cos()];
        q[axis] = (angle * 0.5).sin();
        q
    };
    multiply(multiply(around(2, angles[2]), around(0, angles[0])), around(1, angles[1]))
}

fn name_part(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect()
}

/// The node's name for a trigger. `type_name` is what the game's trigger definitions call the type
pub fn trigger_node_name(index: usize, trigger: &GeTrigger, type_name: Option<&str>, first_spawn: bool) -> String {
    match trigger.type_id {
        // the importer takes one `spawn...`: a second spawn point is named as any other trigger
        TRIGGER_SPAWN if first_spawn => "spawn".to_string(),
        // a team's has data 0 at 1 and the team in data 1 (see `level::multiplayer_spawn_trigger`)
        TRIGGER_MULTIPLAYER_SPAWN => match (trigger.data[0], trigger.data[1]) {
            (Some(1), Some(team)) => format!("mp_spawn_team{team}_{index:03}"),
            _ => format!("mp_spawn_{index:03}"),
        },
        other => match type_name {
            Some(name) => format!("trigger_{index:03}_{}", name_part(name)),
            None => format!("trigger_{index:03}_0x{other:02X}"),
        },
    }
}

fn values<T: Copy + Into<Value>>(list: &[Option<T>]) -> Value {
    let mut map = Map::new();
    for (slot, v) in list.iter().enumerate() {
        if let Some(v) = v {
            map.insert(slot.to_string(), (*v).into());
        }
    }
    Value::Object(map)
}

/// The triggers as the text of a .gltf file. `type_name` names a trigger type, None when it has
/// no name
pub fn export_triggers_gltf(
    triggers: &[GeTrigger],
    type_name: &dyn Fn(u32) -> Option<String>,
) -> anyhow::Result<String> {
    let mut nodes = vec![];
    let mut spawn_seen = false;
    for (index, trigger) in triggers.iter().enumerate() {
        let type_name = type_name(trigger.type_id);
        let first_spawn = trigger.type_id == TRIGGER_SPAWN && !spawn_seen;
        spawn_seen |= first_spawn;

        let [x, y, z] = trigger.position;
        let [qx, qy, qz, qw] = rotation_of(trigger.rotation);
        let mut extras = Map::new();
        extras.insert("trigger_index".into(), index.into());
        extras.insert("trigger_type".into(), trigger.type_id.into());
        if let Some(name) = &type_name {
            extras.insert("trigger_type_name".into(), name.clone().into());
        }
        extras.insert("trigger_subtype".into(), trigger.subtype.into());
        extras.insert("trigger_debug".into(), trigger.debug.into());
        extras.insert("trigger_game_flags".into(), trigger.game_flags.into());
        extras.insert("trigger_rotation".into(), json!(trigger.rotation));
        extras.insert("trigger_data".into(), values(&trigger.data));
        extras.insert("trigger_links".into(), values(&trigger.links));
        extras.insert("trigger_engine".into(), values(&trigger.engine));
        extras.insert("trigger_link_ref".into(), trigger.link_ref.into());

        nodes.push(json!({
            "name": trigger_node_name(index, trigger, type_name.as_deref(), first_spawn),
            "translation": [-x, y, z],
            "rotation": [qx, -qy, -qz, qw],
            "scale": trigger.scale,
            "extras": extras,
        }));
    }

    let root = json!({
        "asset": { "version": "2.0", "generator": "Eurochef" },
        "scene": 0,
        "scenes": [{ "name": "triggers", "nodes": (0..nodes.len()).collect::<Vec<_>>() }],
        "nodes": nodes,
    });
    Ok(serde_json::to_string_pretty(&root)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ge::level::multiplayer_spawn_trigger;

    #[test]
    fn names_are_the_importers() {
        let spawn = GeTrigger::with_defaults(TRIGGER_SPAWN, [1.0, 2.0, 3.0]);
        let anyone = multiplayer_spawn_trigger([0.0; 3], 0.5, None);
        let team = multiplayer_spawn_trigger([0.0; 3], 0.5, Some(1));
        let other = GeTrigger::new(0x3E, [0.0; 3]);
        assert_eq!(trigger_node_name(0, &spawn, None, true), "spawn");
        assert_eq!(trigger_node_name(4, &anyone, None, false), "mp_spawn_004");
        assert_eq!(trigger_node_name(5, &team, None, false), "mp_spawn_team1_005");
        assert_eq!(trigger_node_name(6, &other, Some("TR_LoadMap"), false), "trigger_006_TR_LoadMap");
        assert_eq!(trigger_node_name(7, &other, None, false), "trigger_007_0x3E");
    }

    /// The node's +z, read the way the importer does, gives the trigger's yaw back
    #[test]
    fn yaw_comes_back() {
        let yaw = 0.7f32;
        let text = export_triggers_gltf(&[multiplayer_spawn_trigger([4.0, 1.0, -2.0], yaw, None)], &|_| None).unwrap();
        let root: Value = serde_json::from_str(&text).unwrap();
        let node = &root["nodes"][0];
        let q: Vec<f32> = node["rotation"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
        // +z turned by the quaternion
        let ahead = [2.0 * (q[0] * q[2] + q[3] * q[1]), 1.0 - 2.0 * (q[0] * q[0] + q[1] * q[1])];
        assert!(((-ahead[0]).atan2(ahead[1]) - yaw).abs() < 1e-5);
        assert_eq!(node["translation"][0].as_f64().unwrap(), -4.0);
    }
}
