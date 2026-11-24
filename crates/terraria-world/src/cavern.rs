//! The two cavern species groups are selected from the world ID when the world is loaded.
/// Release 326 uses the subtractive UnifiedRandom generator for this table.
struct WorldRandom {
    state: [i32; 56],
    cursor: usize,
}
fn difference(a: i32, b: i32) -> i32 {
    let d = a.wrapping_sub(b);
    if d < 0 { d + i32::MAX } else { d }
}
impl WorldRandom {
    fn new(seed: i32) -> Self {
        let mut state = [0; 56];
        let mut previous = 161803398 - seed.saturating_abs();
        state[55] = previous;
        let mut value = 1;
        for i in 1..55 {
            state[(21 * i) % 55] = value;
            (previous, value) = (value, difference(previous, value));
        }
        for _ in 0..4 {
            for i in 1..=55 {
                state[i] = difference(state[i], state[1 + (i + 30) % 55]);
            }
        }
        Self { state, cursor: 0 }
    }
    fn next(&mut self, limit: u16) -> u16 {
        self.cursor = 1 + self.cursor % 55;
        let other = 1 + (self.cursor + 20) % 55;
        let v = difference(self.state[self.cursor], self.state[other]).min(i32::MAX - 1);
        self.state[self.cursor] = v;
        (f64::from(v) * (1.0 / f64::from(i32::MAX)) * f64::from(limit)) as u16
    }
}
pub(super) fn cavern_monsters(world_id: i32) -> [u16; 6] {
    let mut rng = WorldRandom::new(world_id);
    let mut first = rng.next(3);
    let second = rng.next(3);
    while first == second {
        first = rng.next(3);
    }
    let species = [(494, 2), (496, 2), (498, 9)];
    std::array::from_fn(|i| {
        let group = if i < 3 { first } else { second };
        let (base, len) = species[usize::from(group)];
        base + rng.next(len)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cavern_groups_match_two_reference_worlds() {
        assert_eq!(cavern_monsters(1904902962), [494, 494, 494, 497, 496, 496]);
        assert_eq!(cavern_monsters(1609884296), [504, 503, 502, 497, 496, 496]);
        assert_eq!(cavern_monsters(i32::MIN), cavern_monsters(i32::MAX));
    }
}
