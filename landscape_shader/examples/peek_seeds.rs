// A quick look at the five named worlds: max height, water, objects, and an ASCII map.
use landscape_shader::game_map::{Triangle, STRONGHOLD_COLS, STRONGHOLD_ROWS};
use landscape_shader::rng::NAMED_WORLD_SEEDS;
use landscape_shader::stronghold_gen::StrongholdRoll;

fn main() {
    for seed in NAMED_WORLD_SEEDS {
        let roll = StrongholdRoll::roll(seed);
        let rivers = roll.river_count();
        let map = roll.into_game_map().expect("valid");
        let c = map.counts();
        println!("seed {seed}: max height {} rivers {rivers} water tris {} trees {} buildings {}", map.max_height(), c.water_triangles, c.trees, c.buildings);
        for y in (0..STRONGHOLD_ROWS as i32).step_by(2) {
            let row: String = (0..STRONGHOLD_COLS as i32).map(|x| {
                let wa = map.is_water(x, y, Triangle::A).unwrap_or(false);
                let wb = map.is_water(x, y, Triangle::B).unwrap_or(false);
                if wa && wb { '~' } else if wa || wb { ',' } else {
                    let h = map.height_at(x, y) as usize;
                    b" .:-=+*#%@"[(h / 13).min(9)] as char
                }
            }).collect();
            println!("{row}");
        }
    }
}
