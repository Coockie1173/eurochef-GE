//! What a base gun's parts and scripts are, to name them in the rig (`weapon_rig`) and in
//! `ge weapon --parts`.
//!
//! The parts were told apart by looking at each one (its mesh alone and on the gun) and at what
//! moves it: o_guns has no names for them. A part is named by its entity, as the same entity is
//! the same mesh in every gun that has it (the AK-47's magazine 820000AF turns up in most rifles'
//! lists), else by the locator it is put on. Copies of a part in a gun's fourth firing mode, with
//! textures of their own, are `_Alt`.
//!
//! A gun plays the scripts of the weapon definitions (DOL 80578AB0, `DEFINITIONS`): each row is a
//! generic script and the gun's own for each of the four firing modes. A gun lists only what
//! differs from its parent, so most rifles play some of the AK-47's scripts (the AK's reload for
//! an action they don't override) and with them the AK's own parts. Those parts are `_from_` the
//! gun whose they are.

use super::weapon::{GunPart, BASE_GUNS};

const BODY: u32 = 0x15000168;

/// The parts by their entity in o_guns (the hash in its entity list)
const PART_NAMES: &[(u32, &str)] = &[
    // P99, and the Torka, Hawksman, Kunara, Wolfe and Golden Gun play its scripts
    (0x02000B72, "Magazine"),
    (0x02000B73, "Magazine_Release"),
    (0x02000B75, "Slide"),
    (0x02000B76, "Silencer"),
    (0x02000B77, "Trigger"),
    // Torka T3
    (0x82000174, "Magazine"),
    (0x82000176, "Slide"),
    (0x82000177, "Trigger"),
    // Hawksman M5A
    (0x8200017D, "Magazine"),
    (0x8200017E, "Hammer"),
    (0x8200017F, "Slide"),
    (0x82000181, "Trigger"),
    // Kunara V
    (0x82000134, "Trigger"),
    (0x82000136, "Magazine"),
    (0x82000137, "Slide"),
    // Wolfe .44
    (0x82000112, "Cylinder"),
    (0x82000113, "Hammer"),
    (0x82000114, "Crane"),
    (0x82000115, "Ejector_Rod"),
    (0x82000119, "Round_1"),
    (0x8200011A, "Round_2"),
    (0x8200011B, "Round_3"),
    (0x82000116, "Round_4"),
    (0x82000117, "Round_5"),
    (0x82000118, "Round_6"),
    (0x8200011D, "Trigger"),
    // AK-47, and most rifles play its scripts
    (0x820000A9, "Charging_Handle"),
    (0x820000AF, "Magazine"),
    (0x820000B0, "Trigger"),
    (0x82000037, "Reload_Magazine"),
    (0x8200003B, "Cleaning_Rod"),
    (0x820000AC, "Front_Sight"),
    (0x820000AE, "Front_Sight_Low"),
    (0x82000183, "Charging_Handle_Alt"),
    (0x82000186, "Magazine_Alt"),
    (0x82000188, "Trigger_Alt"),
    // Sigmus
    (0x8200015D, "Charging_Handle"),
    (0x8200015E, "Magazine"),
    (0x8200015F, "Trigger"),
    (0x82000161, "Front_Sight"),
    (0x82000165, "Front_Sight_Low"),
    (0x8200018B, "Charging_Handle_Alt"),
    (0x8200018D, "Magazine_Alt"),
    (0x82000190, "Trigger_Alt"),
    // Sigmus 9
    (0x820000D0, "Charging_Handle"),
    (0x820000D1, "Selector"),
    (0x820000D2, "Front_Sight"),
    (0x820000D6, "Magazine"),
    (0x820000D7, "Stock"),
    (0x820000D8, "Trigger"),
    (0x820000DA, "Reload_Magazine"),
    // Stauger UA-1
    (0x820000E6, "Charging_Handle"),
    (0x820000E7, "Front_Sight"),
    (0x820000E8, "Magazine"),
    // Vargen FH-7
    (0x820000EE, "Magazine"),
    (0x820000F2, "Small_Details"),
    (0x820000F3, "Trigger"),
    // Strata SV-400
    (0x8200013F, "Rear_Sight"),
    (0x82000141, "Magazine"),
    (0x82000143, "Front_Sight"),
    (0x82000144, "Stock"),
    // SLY 2020
    (0x820000B2, "Magazine"),
    (0x820000B3, "Forend"),
    // SEGS 550, and the SLY 2020 plays its scripts
    (0x820000F6, "Pump"),
    (0x820000F8, "Trigger"),
    (0x82000040, "Shell"),
    (0x82000192, "Pump_Alt"),
    (0x82000194, "Trigger_Alt"),
    // Drumhead Type-12
    (0x82000104, "Stock"),
    // PT-9 Interdictus
    (0x82000155, "Shell"),
    (0x82000156, "Front_Sight"),
    (0x82000157, "Pump"),
    (0x82000158, "Rear_Sight"),
    // Masterton M-557
    (0x82000179, "Charging_Handle"),
    (0x8200017A, "Magazine"),
    // Kallos-TT9
    (0x82000045, "Selector"),
    (0x820000F9, "Charging_Handle"),
    (0x820000FC, "Magazine"),
    (0x820000FE, "Trigger"),
    (0x8200019E, "Charging_Handle_Alt"),
    (0x820001A2, "Magazine_Alt"),
    (0x820001A5, "Trigger_Alt"),
    // Anova DP3, and the Kallos-TT9 plays its scripts
    (0x820000B5, "Charging_Handle"),
    (0x820000B6, "Charging_Handle_2"),
    (0x820000B7, "Selector"),
    (0x820000B8, "Foregrip"),
    (0x820000BB, "Launcher_Barrel"),
    (0x820000BC, "Launcher_Lever"),
    (0x820000BD, "Launcher_Latch"),
    (0x820000BE, "Magazine"),
    (0x820000C3, "Trigger"),
    (0x8200003D, "Grenade"),
    // Terralite III
    (0x8200014B, "Charging_Handle"),
    (0x8200014C, "Magazine"),
    (0x82000149, "Launcher_Barrel"),
    (0x82000150, "Launcher_Mount"),
    (0x82000151, "Front_Sight"),
    // Ivana Spec-R
    (0x820000DE, "Front_Sight"),
    (0x820000E2, "Charging_Handle"),
    (0x820000E4, "Magazine"),
    // Pavlov ASR
    (0x82000108, "Charging_Handle"),
    (0x8200010B, "Magazine"),
    (0x82000110, "Trigger"),
    // Toros AV-400
    (0x8200012D, "Magazine"),
    (0x82000130, "Trigger"),
    // Talon HL 450
    (0x82000167, "Safety"),
    (0x8200016A, "Magazine"),
    (0x82000170, "Trigger"),
    // WA2000
    (0x82000120, "Bolt"),
    (0x82000121, "Magazine"),
    (0x82000128, "Trigger"),
    (0x82000195, "Bolt_Alt"),
    (0x82000196, "Magazine_Alt"),
    (0x8200019B, "Trigger_Alt"),
    (0x8200019D, "Scope_Rail_Alt"),
    // Gambit CP-208
    (0x820000C5, "Bolt"),
    (0x820000C6, "Bolt_Handle"),
    (0x820000C7, "Magazine"),
    (0x820000C8, "Trigger"),
    // MJR-409
    (0x8200012A, "Launcher_Tube"),
];

/// The parts by the locator they are put on, for one `PART_NAMES` doesn't have. The first nine
/// are `ge_gun_parts`' (805730D0: the body, then the attachments by the model's flags), the rest
/// what the guns put there
fn locator_part(locator: u32) -> Option<&'static str> {
    Some(match locator {
        BODY => "Body",
        0x150001CA => "Always_Shown",
        0x150000CD => "Silencer",
        0x150000D7 => "Reflex_Sight",
        0x1500014F => "ACOG_Scope",
        0x15000150 => "Laser_Pointer",
        0x15000157 => "Thermal_Scope",
        0x150000DA => "Grenade_Launcher",
        0x150000DE => "Sniper_Scope",
        0x150000D9 => "Foregrip",
        0x150000DC => "Launcher_Barrel",
        0x150000DD => "Under_Barrel",
        0x15000158 => "Launcher_Lever",
        0x15000181 => "Reload_Part",
        0x15000182 => "Front_Sight_Low",
        0x15000183 => "Front_Sight",
        0x15000187 => "Reload_Part_2",
        0x15000199 => "Rear_Sight",
        0x150001CF => "Launcher_Mount",
        _ => return None,
    })
}

/// What a part is: `Magazine`, `Slide`, `ACOG_Scope`... `Part` for one not told apart yet
pub fn part_name(part: &GunPart) -> &'static str {
    if part.locator == BODY {
        return "Body";
    }
    PART_NAMES
        .iter()
        .find(|(entity, _)| *entity == part.entity_hash)
        .map(|(_, name)| *name)
        .or_else(|| locator_part(part.locator))
        .unwrap_or("Part")
}

/// A base gun's name by its id, as `BASE_GUNS` has it
pub fn gun_name(id: u32) -> Option<&'static str> {
    BASE_GUNS.iter().find(|g| g.1 == id).map(|g| g.0)
}

/// The generic scripts the definitions' rows start with, as the actions they are
fn action(generic: u32) -> String {
    match generic {
        0x04001DAC => "Idle".to_string(),
        0x0400207D => "Fire".to_string(),
        0x04001DAD => "Reload".to_string(),
        // a second reload: the first firing mode's is the reload for most guns, the launcher's
        // mode's the grenade launcher's
        0x04002736 => "Reload_2".to_string(),
        _ => format!("Action_{:04X}", generic & 0xFFFF),
    }
}

/// The firing modes: the first, the grenade launcher's (only the guns that carry one have it),
/// a third the pistols fill with the first's, and a fourth some guns have with parts of their own
const MODES: [&str; 4] = ["", "_Launcher", "_Mode3", "_Alt"];

/// What a script of a gun is to it: the action it plays it for (`Reload`, `Fire_Alt`...) and the
/// gun whose definition it is in, the gun itself or the parent it has it from. As the game finds
/// them: the gun's rows first, then each parent's for an action the nearer ones don't have
pub fn script_role(gun: u32, script: u32) -> Option<(String, u32)> {
    let mut seen: Vec<u32> = vec![];
    let mut at = Some(gun);
    for _ in 0..8 {
        let Some((id, parent, rows)) = at.and_then(|id| DEFINITIONS.iter().find(|d| d.0 == id)) else {
            break;
        };
        for row in rows.iter() {
            if seen.contains(&row[0]) {
                continue;
            }
            seen.push(row[0]);
            if let Some(mode) = (1..5).find(|&k| row[k] == script) {
                return Some((format!("{}{}", action(row[0]), MODES[mode - 1]), *id));
            }
        }
        at = Some(*parent);
    }
    None
}

/// The weapon definitions (DOL 80578AB0, the ones with a script list +0x1C): id, parent, and
/// rows of the generic script and the gun's own for each firing mode (0: the generic's own,
/// as the parent has it)
pub const DEFINITIONS: &[(u32, u32, &[[u32; 5]])] = &[
    (0x0200016E, 0x02000001, &[[0x04001DAC, 0x04001DAC, 0x00000000, 0x04001DAC, 0x00000000], [0x0400207D, 0x0400207D, 0x00000000, 0x0400207D, 0x00000000], [0x04001DAD, 0x04001DAD, 0x00000000, 0x04001DAD, 0x00000000], [0x04001DEC, 0x04001DEC, 0x00000000, 0x00000000, 0x00000000], [0x04001DED, 0x04001DED, 0x00000000, 0x00000000, 0x00000000], [0x0400286E, 0x0400286E, 0x00000000, 0x0400286E, 0x00000000], [0x04002872, 0x04002872, 0x00000000, 0x04002872, 0x00000000], [0x04002736, 0x04002736, 0x00000000, 0x00000000, 0x00000000]]),
    (0x0200092E, 0x0200016E, &[[0x04001DAC, 0x04002537, 0x00000000, 0x04002537, 0x00000000], [0x0400207D, 0x04002541, 0x00000000, 0x04002541, 0x00000000], [0x04001DAD, 0x04002538, 0x00000000, 0x04002538, 0x00000000], [0x0400286E, 0x04002C27, 0x00000000, 0x04002C27, 0x00000000], [0x04002872, 0x04002C24, 0x00000000, 0x04002C24, 0x00000000]]),
    (0x0200031D, 0x0200016E, &[[0x04001DAC, 0x0400254B, 0x00000000, 0x0400254B, 0x00000000], [0x0400207D, 0x0400254C, 0x00000000, 0x0400254C, 0x00000000], [0x04001DAD, 0x0400254D, 0x00000000, 0x0400254D, 0x00000000], [0x0400286E, 0x04002C28, 0x00000000, 0x04002C28, 0x00000000], [0x04002872, 0x04002C26, 0x00000000, 0x04002C26, 0x00000000]]),
    (0x0200092F, 0x0200016E, &[[0x04001DAC, 0x04002548, 0x00000000, 0x04002548, 0x00000000], [0x0400207D, 0x04002549, 0x00000000, 0x04002549, 0x00000000], [0x04001DAD, 0x0400254A, 0x00000000, 0x0400254A, 0x00000000], [0x0400286E, 0x04002C29, 0x00000000, 0x04002C29, 0x00000000], [0x04002872, 0x04002C2A, 0x00000000, 0x04002C2A, 0x00000000]]),
    (0x0200074E, 0x0200016E, &[[0x04001DAC, 0x0400213E, 0x00000000, 0x0400213E, 0x00000000], [0x0400207D, 0x0400213F, 0x00000000, 0x0400213F, 0x00000000], [0x04001DAD, 0x04002141, 0x00000000, 0x04002141, 0x00000000]]),
    (0x02000827, 0x0200016E, &[[0x04001DAC, 0x0400256D, 0x00000000, 0x0400256D, 0x00000000], [0x0400207D, 0x0400256E, 0x00000000, 0x0400256E, 0x00000000], [0x04001DAD, 0x0400256F, 0x00000000, 0x0400256F, 0x00000000]]),
    (0x02000930, 0x0200031E, &[[0x04001DAC, 0x0400256A, 0x00000000, 0x00000000, 0x04002F79], [0x0400207D, 0x0400256B, 0x00000000, 0x00000000, 0x04002F7A], [0x04001DAD, 0x0400256C, 0x00000000, 0x00000000, 0x04002F7B]]),
    (0x0200031E, 0x02000319, &[[0x04001DAC, 0x04002186, 0x00000000, 0x00000000, 0x00000000], [0x0400207D, 0x04002263, 0x00000000, 0x00000000, 0x00000000], [0x04001DAD, 0x04002187, 0x00000000, 0x00000000, 0x00000000]]),
    (0x02000931, 0x0200048A, &[[0x04001DAC, 0x0400255A, 0x00000000, 0x00000000, 0x00000000], [0x0400207D, 0x04002559, 0x00000000, 0x00000000, 0x00000000], [0x04001DAD, 0x04002558, 0x00000000, 0x00000000, 0x00000000]]),
    (0x0200048A, 0x02000319, &[[0x04001DAC, 0x040020EA, 0x00000000, 0x00000000, 0x00000000], [0x0400207D, 0x04002B85, 0x00000000, 0x00000000, 0x00000000], [0x04001DAD, 0x040020E9, 0x00000000, 0x00000000, 0x00000000]]),
    (0x02000932, 0x02000319, &[[0x04001DAC, 0x04002567, 0x00000000, 0x00000000, 0x00000000], [0x0400207D, 0x04002568, 0x00000000, 0x00000000, 0x00000000], [0x04001DAD, 0x04002569, 0x00000000, 0x00000000, 0x00000000]]),
    (0x02000933, 0x0200074F, &[[0x04001DAC, 0x0400255E, 0x00000000, 0x00000000, 0x00000000], [0x0400207D, 0x0400255F, 0x00000000, 0x00000000, 0x00000000], [0x04001DAD, 0x04002560, 0x00000000, 0x00000000, 0x00000000]]),
    (0x0200074F, 0x02000319, &[[0x04001DAC, 0x040020EB, 0x00000000, 0x00000000, 0x04002F7C], [0x0400207D, 0x0400212C, 0x00000000, 0x00000000, 0x04002F7D], [0x04001DAD, 0x040021A1, 0x00000000, 0x00000000, 0x04002F7E], [0x04002198, 0x04002152, 0x00000000, 0x00000000, 0x04002F80], [0x04002199, 0x04002150, 0x00000000, 0x00000000, 0x04002F7F]]),
    (0x02000934, 0x0200048B, &[[0x04001DAC, 0x04002453, 0x00000000, 0x00000000, 0x00000000], [0x0400207D, 0x04002454, 0x00000000, 0x00000000, 0x00000000], [0x04001DAD, 0x04002455, 0x00000000, 0x00000000, 0x00000000], [0x04002198, 0x04002557, 0x00000000, 0x00000000, 0x00000000], [0x04002199, 0x04002456, 0x00000000, 0x00000000, 0x00000000]]),
    (0x02000935, 0x0200048B, &[[0x04001DAC, 0x04002561, 0x00000000, 0x00000000, 0x00000000], [0x0400207D, 0x04002562, 0x00000000, 0x00000000, 0x00000000], [0x04001DAD, 0x04002563, 0x00000000, 0x00000000, 0x00000000], [0x04002198, 0x04002565, 0x00000000, 0x00000000, 0x00000000], [0x04002199, 0x04002564, 0x00000000, 0x00000000, 0x00000000]]),
    (0x02000936, 0x02000319, &[[0x04001DAC, 0x04002554, 0x00000000, 0x00000000, 0x00000000], [0x0400207D, 0x04002555, 0x00000000, 0x00000000, 0x00000000], [0x04001DAD, 0x04002556, 0x00000000, 0x00000000, 0x00000000]]),
    (0x02000319, 0x02000001, &[[0x04001DAC, 0x04001DB3, 0x04001DB3, 0x00000000, 0x04002F74], [0x0400207D, 0x04002B83, 0x04002B83, 0x00000000, 0x04002F75], [0x04001DAD, 0x04001DF1, 0x04001DF1, 0x00000000, 0x04002F78], [0x04002736, 0x04001DF1, 0x04002634, 0x00000000, 0x04002F76]]),
    (0x02000937, 0x02000489, &[[0x04001DAC, 0x04002551, 0x04002551, 0x00000000, 0x04002FBA], [0x0400207D, 0x04002553, 0x04002553, 0x00000000, 0x04002FBB], [0x04001DAD, 0x04002552, 0x04002552, 0x00000000, 0x04002FBD], [0x04002736, 0x04002552, 0x040026B4, 0x00000000, 0x04002FBC]]),
    (0x02000489, 0x02000319, &[[0x04001DAC, 0x0400207B, 0x0400207B, 0x00000000, 0x00000000], [0x0400207D, 0x040020CF, 0x040020CF, 0x00000000, 0x00000000], [0x04001DAD, 0x0400207C, 0x0400207C, 0x00000000, 0x00000000], [0x04002736, 0x0400207C, 0x04002218, 0x00000000, 0x00000000], [0x04001DEC, 0x040021BA, 0x040021BA, 0x00000000, 0x00000000], [0x04001DED, 0x040021B9, 0x040021B9, 0x00000000, 0x00000000]]),
    (0x0200031F, 0x02000319, &[[0x04001DAC, 0x0400254F, 0x0400254F, 0x00000000, 0x00000000], [0x0400207D, 0x0400254E, 0x0400254E, 0x00000000, 0x00000000], [0x04001DAD, 0x04002550, 0x04002550, 0x00000000, 0x00000000], [0x04002736, 0x04002550, 0x040026F7, 0x00000000, 0x00000000]]),
    (0x02000938, 0x02000319, &[[0x04001DAC, 0x04002577, 0x00000000, 0x00000000, 0x00000000], [0x0400207D, 0x04002578, 0x00000000, 0x00000000, 0x00000000], [0x04001DAD, 0x04002579, 0x00000000, 0x00000000, 0x00000000]]),
    (0x02000750, 0x02000319, &[[0x04001DAC, 0x040020E7, 0x00000000, 0x00000000, 0x00000000], [0x0400207D, 0x04002B84, 0x00000000, 0x00000000, 0x00000000], [0x04001DAD, 0x040020E8, 0x00000000, 0x00000000, 0x00000000]]),
    (0x02000939, 0x02000319, &[[0x04001DAC, 0x04002570, 0x00000000, 0x00000000, 0x00000000], [0x0400207D, 0x04002571, 0x00000000, 0x00000000, 0x00000000], [0x04001DAD, 0x04002572, 0x00000000, 0x00000000, 0x00000000]]),
    (0x0200093A, 0x02000319, &[[0x04001DAC, 0x0400255C, 0x00000000, 0x00000000, 0x00000000], [0x0400207D, 0x0400255D, 0x00000000, 0x00000000, 0x00000000], [0x04001DAD, 0x04002566, 0x00000000, 0x00000000, 0x00000000]]),
    (0x0200048B, 0x02000319, &[[0x04001DAC, 0x04001DFD, 0x00000000, 0x00000000, 0x04002F82], [0x0400207D, 0x04002105, 0x00000000, 0x00000000, 0x04002F83], [0x04001DAD, 0x04001DFE, 0x00000000, 0x00000000, 0x04002F84]]),
    (0x0200077C, 0x02000319, &[[0x04001DAC, 0x040021B7, 0x00000000, 0x00000000, 0x00000000], [0x0400207D, 0x0400255B, 0x00000000, 0x00000000, 0x00000000], [0x04001DAD, 0x040021B8, 0x00000000, 0x00000000, 0x00000000]]),
    (0x02000828, 0x02000319, &[[0x04001DAC, 0x04002405, 0x00000000, 0x00000000, 0x00000000], [0x0400207D, 0x04002642, 0x00000000, 0x00000000, 0x00000000], [0x04001DAD, 0x04002406, 0x00000000, 0x00000000, 0x00000000], [0x04002736, 0x04002406, 0x00000000, 0x00000000, 0x00000000], [0x0400286E, 0x04002CAB, 0x00000000, 0x00000000, 0x00000000]]),
];
