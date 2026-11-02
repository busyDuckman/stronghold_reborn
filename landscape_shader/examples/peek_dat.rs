// Print the size and kind of a few STRONG.DAT entry runs, to see how the sheets were cut.
use landscape_shader::dat::StrongDat;

fn main() {
    let Some(path) = StrongDat::find() else { println!("no STRONG.DAT"); return };
    let dat = StrongDat::open(&path).expect("opens");
    for (label, range) in [("tree01", 300..312), ("tree02", 312..324), ("rock01", 372..381), ("geninn01", 36..45), ("1fi1m", 1170..1200), ("orcs", 1979..2009)] {
        print!("{label}:");
        for i in range {
            match dat.image(i) {
                Ok(im) => print!(" {i}:k{}:{}x{}@({},{})", im.kind, im.width, im.height, im.anchor_x, im.anchor_y),
                Err(_) => print!(" {i}:-"),
            }
        }
        println!();
    }
    println!("palettes: {:?}", dat.palette_indices());
}
