//! Calendar state shared by saved worlds and the direct-IP WorldData packet.
use crate::{Error, World};
use terraria_binary::Reader;
use terraria_protocol::WorldData;

impl World {
    /// Effective day rate from the persistent per-world creative powers section.
    pub fn saved_time_rate(&self) -> Result<u32, Error> {
        let mut r = Reader(&self.data[self.offsets[9]..self.offsets[10]]);
        let mut frozen = false;
        let mut rate = 1;
        while r.byte()? != 0 {
            match r.u16()? {
                0 => frozen = r.byte()? != 0,
                8 => {
                    let slider = r.f32()?;
                    if !slider.is_finite() {
                        return Err(Error::Invalid("non-finite creative time slider"));
                    }
                    rate = (1.0 + 23.0 * slider.clamp(0.0, 1.0)).round_ties_even() as u32;
                }
                9 | 10 | 13 => {
                    r.byte()?;
                }
                12 => {
                    r.f32()?;
                }
                _ => return Err(Error::Invalid("unknown persistent creative power")),
            }
        }
        if !r.0.is_empty() {
            return Err(Error::Invalid("trailing creative powers"));
        }
        Ok(if frozen { 0 } else { rate })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WorldClock {
    pub time: f64,
    pub daytime: bool,
    pub moon_phase: u8,
    pub blood_moon: bool,
    pub eclipse: bool,
    pub sundial: u8,
    pub moondial: u8,
    pub fast_dawn: bool,
    pub fast_dusk: bool,
}

impl WorldClock {
    pub fn activate_dial(&mut self, dusk: bool) -> bool {
        let (cooldown, fast) = if dusk {
            (&mut self.moondial, &mut self.fast_dusk)
        } else {
            (&mut self.sundial, &mut self.fast_dawn)
        };
        if *cooldown != 0 {
            return false;
        }
        *cooldown = 8;
        *fast = true;
        true
    }

    pub fn from_world(time: f64, info: &WorldData) -> Self {
        Self {
            time,
            daytime: info.time_flags & 1 != 0,
            moon_phase: info.moon_phase,
            blood_moon: info.time_flags & 2 != 0,
            eclipse: info.time_flags & 4 != 0,
            sundial: info.sundial,
            moondial: info.moondial,
            fast_dawn: info.progression[2] & 2 != 0,
            fast_dusk: info.progression[8] & 128 != 0,
        }
    }
    pub fn apply(&self, info: &mut WorldData) {
        info.time = self.time as i32;
        info.time_flags = (info.time_flags & !7)
            | u8::from(self.daytime)
            | (u8::from(self.blood_moon) << 1)
            | (u8::from(self.eclipse) << 2);
        info.moon_phase = self.moon_phase;
        info.sundial = self.sundial;
        info.moondial = self.moondial;
        info.progression[2] = (info.progression[2] & !2) | (u8::from(self.fast_dawn) << 1);
        info.progression[8] = (info.progression[8] & !128) | (u8::from(self.fast_dusk) << 7);
    }
    /// Advance one simulation step. The reference changes phase strictly after its final tick.
    pub fn tick(&mut self, rate: u32) -> bool {
        let limit = if self.daytime { 54000. } else { 32400. };
        self.time += if self.fast_dawn || self.fast_dusk {
            60.
        } else {
            f64::from(rate)
        };
        if self.time <= limit {
            return false;
        }
        self.time = 0.;
        self.daytime = !self.daytime;
        self.blood_moon = false;
        self.eclipse = false;
        if self.daytime {
            self.moon_phase = (self.moon_phase.wrapping_add(1)) % 8;
            self.sundial = self.sundial.saturating_sub(1);
            self.fast_dawn = false;
        } else {
            self.moondial = self.moondial.saturating_sub(1);
            self.fast_dusk = false;
        }
        true
    }
}

pub(crate) struct ClockOffsets {
    pub time: usize,
    pub sundial: usize,
    pub dawn: usize,
    pub moondial: usize,
}

impl ClockOffsets {
    pub fn write(&self, out: &mut [u8], clock: WorldClock) {
        let at = self.time;
        out[at..at + 8].copy_from_slice(&clock.time.to_le_bytes());
        out[at + 8] = u8::from(clock.daytime);
        out[at + 9..at + 13].copy_from_slice(&i32::from(clock.moon_phase).to_le_bytes());
        out[at + 13] = u8::from(clock.blood_moon);
        out[at + 14] = u8::from(clock.eclipse);
        out[self.sundial] = clock.sundial;
        out[self.dawn] = u8::from(clock.fast_dawn);
        out[self.moondial - 1] = u8::from(clock.fast_dusk);
        out[self.moondial] = clock.moondial;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> World {
        World::parse(include_bytes!("../../../docs/fixtures/entry-326.wld").to_vec()).unwrap()
    }

    #[test]
    fn saved_powers_use_clamped_slider_bankers_rounding_and_freeze_precedence() {
        let mut world = fixture();
        let start = world.offsets[9];
        assert_eq!(world.saved_time_rate().unwrap(), 1);
        for (slider, expected) in [(-1f32, 1), (0.5, 12), (1., 24), (2., 24)] {
            world.data[start + 7..start + 11].copy_from_slice(&slider.to_le_bytes());
            assert_eq!(world.saved_time_rate().unwrap(), expected);
        }
        world.data[start + 3] = 1;
        assert_eq!(world.saved_time_rate().unwrap(), 0);
        world.data[start + 7..start + 11].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(world.saved_time_rate().is_err());
        world.data[start + 1] = 255;
        assert!(world.saved_time_rate().is_err());
    }

    #[test]
    fn both_dials_reach_their_target_and_the_entire_clock_round_trips() {
        let world = fixture();
        for daytime in [false, true] {
            let limit = if daytime { 54000. } else { 32400. };
            let mut clock = WorldClock {
                time: limit - 60.,
                daytime,
                moon_phase: 7,
                eclipse: daytime,
                blood_moon: !daytime,
                sundial: 3,
                moondial: 4,
                fast_dawn: !daytime,
                fast_dusk: daytime,
            };
            // Fast-forward takes precedence over frozen time. Exactly at the limit stays in phase.
            assert!(!clock.tick(0));
            assert_eq!(clock.time, limit);
            assert!(clock.tick(0));
            assert_eq!(clock.time, 0.);
            assert_eq!(clock.daytime, !daytime);
            assert_eq!(
                (clock.moon_phase, clock.sundial, clock.moondial),
                if daytime { (7, 3, 3) } else { (0, 2, 4) }
            );
            assert!(!clock.eclipse && !clock.blood_moon && !clock.fast_dawn && !clock.fast_dusk);
            assert!(!clock.tick(0));
            assert_eq!(clock.time, 0.);
            clock.time = 17.5;
            assert!(!clock.tick(12));
            let saved = world
                .checkpoint(&Default::default(), &Default::default(), clock)
                .unwrap();
            let info = saved.world_data().unwrap();
            assert_eq!(info.time, 29);
            assert_eq!(WorldClock::from_world(saved.metadata().time, &info), clock);
        }
    }
}
