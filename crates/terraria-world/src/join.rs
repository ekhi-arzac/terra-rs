//! Project the saved world header into the state sent before the first game tick.
use super::{Error, World};
use terraria_binary::Reader;
use terraria_protocol::WorldData;

fn bits(flags: [bool; 8]) -> u8 {
    flags
        .into_iter()
        .enumerate()
        .fold(0, |mask, (i, flag)| mask | (u8::from(flag) << i))
}

fn bools<const N: usize>(r: &mut Reader<'_>) -> Result<[bool; N], Error> {
    Ok(r.take::<N>()?.map(|b| b != 0))
}

fn count(value: i32) -> Result<usize, Error> {
    usize::try_from(value).map_err(|_| Error::Invalid("negative world header array count"))
}

pub struct JoinState {
    pub info: WorldData,
    pub(crate) clock_offsets: crate::clock::ClockOffsets,
    pub banners: terraria_protocol::BannerState,
    pub shields: [u16; 4],
    pub cavern_monsters: [u16; 6],
}
impl World {
    pub fn world_data(&self) -> Result<WorldData, Error> {
        Ok(self.join_state()?.info)
    }
    pub fn join_state(&self) -> Result<JoinState, Error> {
        let meta = self.metadata();
        let mut r = Reader(&self.data[self.flags.clone()]);
        let game_mode = r.i32()? as u8;
        let [
            drunk,
            worthy,
            anniversary,
            constant,
            bees,
            remix,
            no_traps,
            zenith,
            skyblock,
        ] = bools(&mut r)?;
        r.take::<16>()?; // Creation and last-save timestamps.
        let moon_type = r.byte()?;
        let tree_x = r.i32s()?;
        let tree_styles = r.i32s::<4>()?.map(|v| v as u8);
        let cave_x = r.i32s()?;
        let cave_styles = r.i32s::<4>()?.map(|v| v as u8);
        let biome_backgrounds = r.i32s::<3>()?.map(|v| v as u8);
        let spawn = r.i32s::<2>()?.map(|v| v as i16);
        let layers = [r.f64()? as i16, r.f64()? as i16];
        let clock_start = self.flags.end - r.0.len();
        let time = r.f64()? as i32;
        let [day] = bools(&mut r)?;
        let moon_phase = r.i32()? as u8;
        let [blood, eclipse] = bools(&mut r)?;
        let dungeon = r.i32s::<2>()?.map(|v| v as i16);
        let [
            crimson,
            eye,
            evil_boss,
            skeletron,
            bee,
            mech1,
            mech2,
            mech3,
            mech_any,
            plant,
            golem,
            king_slime,
        ] = bools(&mut r)?;
        r.take::<3>()?; // Rescued goblin, wizard, mechanic.
        let [goblins, clown, frost, pirates, orb] = bools(&mut r)?;
        r.take::<6>()?; // Pending meteor, orb count, altar count.
        let [hardmode] = bools(&mut r)?;
        r.take::<9>()?; // After-party flag, invasion delay and size.
        let invasion = r.i32()? as i8;
        r.f64()?; // Invasion X.
        let slime_rain_time = r.f64()?;
        let sundial_offset = self.flags.end - r.0.len();
        let sundial = r.byte()?;
        let [raining] = bools(&mut r)?;
        r.i32()?; // Rain duration.
        let max_rain = r.f32()?;
        let hardmode_ores = r.i32s::<3>()?;
        let [
            tree1,
            corrupt,
            jungle,
            snow,
            hallow,
            crimson_bg,
            desert,
            ocean,
        ] = r.take()?;
        r.i32()?; // Vanilla replaces cloudBGActive with a negative timer on load.
        let clouds = r.i16()? as u8;
        let wind = r.f32()?;
        let anglers = count(r.i32()?)?;
        for _ in 0..anglers {
            r.string()?;
        }
        r.take::<16>()?; // Rescues, daily quest, initial invasion size and cultist delay.
        let mut banners = terraria_protocol::BannerState::default();
        let kills = count(r.i16()?.into())?;
        for i in 0..kills {
            let n = r.i32()?;
            if let Some(entry) = banners.kills.get_mut(i) {
                *entry = n;
            }
        }
        let claimable = count(r.i16()?.into())?;
        for i in 0..claimable {
            let n = r.u16()?;
            if let Some(entry) = banners.claimable.get_mut(i) {
                *entry = n;
            }
        }
        let dawn_offset = self.flags.end - r.0.len();
        let [
            dawn,
            fishron,
            martians,
            cultist,
            moonlord,
            pumpking,
            wood,
            ice_queen,
            santank,
            everscream,
            solar,
            vortex,
            nebula,
            stardust,
        ] = bools(&mut r)?;
        let active_towers = bools::<4>(&mut r)?;
        r.byte()?; // Lunar apocalypse.
        let [party_manual, party_genuine] = bools(&mut r)?;
        r.i32()?; // Party cooldown.
        let guests = count(r.i32()?)?;
        r.bytes(
            guests
                .checked_mul(4)
                .ok_or(Error::Invalid("party guest count"))?,
        )?;
        let [sandstorm_active] = bools(&mut r)?;
        r.i32()?;
        r.f32()?; // Time left and current severity.
        let sandstorm = r.f32()?;
        r.byte()?; // Rescued bartender.
        let [dd1, dd2, dd3] = bools(&mut r)?;
        let [mushroom, underworld, tree2, tree3, tree4] = r.take()?;
        let [book] = bools(&mut r)?;
        r.i32()?;
        let [lantern_genuine, lantern_manual, _next_night] = bools(&mut r)?;
        let top_count = count(r.i32()?)?;
        let mut tree_tops = [0; 13];
        for top in tree_tops.iter_mut().take(top_count) {
            *top = r.i32()? as u8;
        }
        let [halloween, xmas] = bools(&mut r)?;
        let base_ores = r.i32s::<4>()?;
        let ores = [
            base_ores[0],
            base_ores[1],
            base_ores[2],
            base_ores[3],
            hardmode_ores[0],
            hardmode_ores[1],
            hardmode_ores[2],
        ]
        .map(|v| v as i16);
        let [cat, dog, bunny, empress, queen_slime, deer, blue] = bools(&mut r)?;
        r.take::<4>()?; // Unlocked merchant, demolitionist, party girl and dye trader.
        let [truffle] = bools(&mut r)?;
        r.take::<3>()?; // Arms dealer, nurse and princess.
        let [
            book2,
            satchel,
            green,
            old,
            purple,
            rainbow,
            red,
            yellow,
            copper,
            dusk,
        ] = bools(&mut r)?;
        let moondial_offset = self.flags.end - r.0.len();
        let moondial = r.byte()?;
        let [permanent_halloween, permanent_xmas, vampire, infected] = bools(&mut r)?;
        r.take::<8>()?; // Meteor shower and coin rain counters.
        let [teams] = bools(&mut r)?;
        let spawn_count = r.byte()?;
        let extra_spawns = (0..spawn_count)
            .map(|_| r.i16s())
            .collect::<Result<_, _>>()?;
        let [dual, more_lightning, no_lightning] = bools(&mut r)?;
        let no_spiders = meta.file_version >= 328 && r.byte()? != 0;
        r.string()?; // World-generation manifest.
        let progression = [
            bits([
                orb, eye, evil_boss, skeletron, hardmode, clown, false, plant,
            ]),
            bits([mech1, mech2, mech3, mech_any, false, crimson, false, false]),
            bits([
                false,
                dawn,
                slime_rain_time > 0.0,
                king_slime,
                bee,
                fishron,
                martians,
                cultist,
            ]),
            bits([
                moonlord,
                pumpking,
                wood,
                ice_queen,
                santank,
                everscream,
                golem,
                party_manual || party_genuine,
            ]),
            bits([
                pirates,
                frost,
                goblins,
                sandstorm_active,
                false,
                dd1,
                dd2,
                dd3,
            ]),
            bits([
                book,
                lantern_genuine || lantern_manual,
                solar,
                vortex,
                nebula,
                stardust,
                halloween,
                xmas,
            ]),
            bits([cat, dog, bunny, false, drunk, empress, queen_slime, worthy]),
            bits([
                anniversary,
                constant,
                deer,
                bees,
                remix,
                blue,
                book2,
                satchel,
            ]),
            bits([green, old, purple, rainbow, red, yellow, copper, dusk]),
            bits([
                no_traps, zenith, truffle, vampire, infected, teams, skyblock, dual,
            ]),
            bits([
                skyblock && (self.active_interior as f32 / (meta.width * meta.height) as f32) < 0.1,
                permanent_halloween,
                permanent_xmas,
                more_lightning,
                no_lightning,
                no_spiders,
                false,
                false,
            ]),
        ];
        let info = WorldData {
            time,
            time_flags: bits([day, blood, eclipse, false, false, false, false, false]),
            moon_phase,
            size: [meta.width as i16, meta.height as i16],
            spawn,
            layers,
            id: meta.world_id,
            name: meta.name.clone(),
            game_mode,
            guid: meta.guid_bytes,
            generator_version: meta.generator_version,
            moon_type,
            backgrounds: [
                tree1, tree2, tree3, tree4, corrupt, jungle, snow, hallow, crimson_bg, desert,
                ocean, mushroom, underworld,
            ],
            biome_backgrounds,
            wind,
            clouds,
            tree_x,
            tree_styles,
            cave_x,
            cave_styles,
            tree_tops,
            rain: if raining { max_rain } else { 0.0 },
            progression,
            sundial,
            moondial,
            ores,
            invasion,
            lobby: 0,
            sandstorm,
            extra_spawns,
            dungeon: if self.has_dungeon { dungeon } else { [-1, -1] },
        };
        let shields = active_towers.map(|active| {
            if active {
                if moonlord { 50 } else { 100 }
            } else {
                0
            }
        });
        Ok(JoinState {
            clock_offsets: crate::clock::ClockOffsets {
                time: clock_start,
                sundial: sundial_offset,
                dawn: dawn_offset,
                moondial: moondial_offset,
            },
            info,
            banners,
            shields,
            cavern_monsters: crate::cavern::cavern_monsters(meta.world_id),
        })
    }
}
