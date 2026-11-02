// The evidence behind src/backdrop.rs, printed: for each of the nine sky sets, which palette
// slots the two images use (they stop at 207, and nearly fill 160..=207 where the 48-colour
// piece is stamped), and how well the images' edges join in each order against unrelated
// columns (the two are the halves of one loop).
use landscape_shader::backdrop::{FIRST_SET_ENTRY, SET_COUNT};
use landscape_shader::dat::StrongDat;

fn column(px: &[u8], w: usize, h: usize, x: usize) -> Vec<u8> {
    (0..h).map(|y| px[y * w + x]).collect()
}

fn same(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).filter(|(x, y)| x == y).count()
}

fn main() {
    let Some(path) = StrongDat::find() else { println!("no STRONG.DAT"); return };
    let dat = StrongDat::open(&path).expect("opens");
    for set in 0..SET_COUNT {
        let first = FIRST_SET_ENTRY + set * 3;
        let a = dat.image(first).expect("first half");
        let b = dat.image(first + 2).expect("second half");
        let piece = dat.payload(first + 1).expect("piece");
        let (w, h) = (a.width as usize, a.height as usize);
        let mut used = [false; 256];
        for p in a.pixels.iter().chain(&b.pixels) {
            used[*p as usize] = true;
        }
        let highest = used.iter().rposition(|&u| u).unwrap_or(0);
        let in_piece = used[160..208].iter().filter(|&&u| u).count();
        let a_left = column(&a.pixels, w, h, 0);
        let a_right = column(&a.pixels, w, h, w - 1);
        let b_left = column(&b.pixels, w, h, 0);
        let b_right = column(&b.pixels, w, h, w - 1);
        let unrelated = column(&a.pixels, w, h, w / 2);
        println!(
            "set {set} entries {first}..={}: {}x{} + {} byte piece; highest slot {highest}, {in_piece} of 48 piece slots used; seams a|b {} b|a {} unrelated {} of {h} rows",
            first + 2, w, h, piece.len(), same(&a_right, &b_left), same(&b_right, &a_left), same(&unrelated, &b_left)
        );
    }
}
