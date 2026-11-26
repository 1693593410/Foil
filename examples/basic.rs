use Foil::Foil;

fn main() {
    let mut world = Foil::new();
    world
        .add(1, 0.0, 0.0, 32.0, 32.0, "rect", false, 0, 0)
        .expect("墙");
    world
        .add(2, 0.0, 0.0, 8.0, 8.0, "circle", true, 1, 1)
        .expect("检测");
    println!("碰上 {:?}", world.overlaps(2));
}
