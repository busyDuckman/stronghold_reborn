// --------------------------------------------------------------------------------------------------------------------
// The original game's random number generator, ported exactly.
//
// Every random decision in the 1993 game comes from one six byte additive shift register.
// Seeding it with one of the five named-world constants and drawing from it in the same
// order the game does reproduces the game's own maps, which is the whole point of having it.
// The five named worlds seed it with 100, 200, 300, 400 and 1026093; "Random" seeds it with
// the clock.

/// The seeds behind the game's five named worlds, in menu order.
pub const NAMED_WORLD_SEEDS: [u32; 5] = [100, 200, 300, 400, 1_026_093];

/// The generator. Its whole state is these six bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct GameRng {
    state: [u8; 6],
}

impl std::fmt::Debug for GameRng {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GameRng({})", self.state_hex())
    }
}

impl GameRng {
    /// Seed from a 32-bit number the way the game does: the four seed bytes go into the middle
    /// of the register each XORed with 0x55, the ends are cleared, and two hundred draws are
    /// thrown away to stir it.
    pub fn seeded(seed: u32) -> GameRng {
        let [b0, b1, b2, b3] = seed.to_le_bytes();
        let mut rng = GameRng { state: [0, b0 ^ 0x55, b1 ^ 0x55, b2 ^ 0x55, b3 ^ 0x55, 0] };
        for _ in 0..200 {
            rng.next_byte();
        }
        rng
    }

    /// One byte, and one step of the register.
    ///   - Adds bytes 1, 4 and 5 with a carry-in of one, in two eight bit adds whose first
    ///     carry feeds the second, exactly as the original does it.
    ///   - Stores the result at byte 0, then shifts every byte up one; byte 0 keeps the result.
    pub fn next_byte(&mut self) -> u8 {
        let first = self.state[1] as u16 + self.state[4] as u16 + 1;
        let second = (first & 0xff) + self.state[5] as u16 + (first >> 8);
        let made = second as u8;
        self.state[0] = made;
        for i in (1..6).rev() {
            self.state[i] = self.state[i - 1];
        }
        made
    }

    /// One byte scaled onto 0 .. n-1: (byte * n) >> 8 in sixteen bits. Never answers n itself.
    pub fn below(&mut self, n: u16) -> u16 {
        ((self.next_byte() as u16).wrapping_mul(n)) >> 8
    }

    /// The six state bytes as the verifier prints them.
    pub fn state_hex(&self) -> String {
        self.state.iter().map(|b| format!("{b:02x}")).collect()
    }
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_100_makes_the_captured_worlds_first_two_draws() {
        // The captured seed-100 world shows the roll's first draw (below 3) came out 1 and the
        // river count (below 20) came out 16.
        let mut rng = GameRng::seeded(100);
        assert_eq!(rng.below(3), 1);
        assert_eq!(rng.below(0x14), 16);
    }

    #[test]
    fn below_never_answers_n() {
        let mut rng = GameRng::seeded(7);
        for _ in 0..5000 {
            assert!(rng.below(4) < 4);
            assert!(rng.below(0x94) < 0x94);
        }
    }

    #[test]
    fn same_seed_same_stream() {
        let mut a = GameRng::seeded(300);
        let mut b = GameRng::seeded(300);
        let mut c = GameRng::seeded(301);
        let sa: Vec<u8> = (0..64).map(|_| a.next_byte()).collect();
        let sb: Vec<u8> = (0..64).map(|_| b.next_byte()).collect();
        let sc: Vec<u8> = (0..64).map(|_| c.next_byte()).collect();
        assert_eq!(sa, sb);
        assert_ne!(sa, sc);
    }
}
