use super::*;

    fn put(w: &mut Foil, idx: usize, x: f32, y: f32, ww: f32, h: f32, shape: &str, sensor: bool) {
        w.add(idx, x as f64, y as f64, ww as f64, h as f64, shape, sensor, 0, 0).unwrap();
    }

    #[test]
    fn sensor_sees_near_wall_and_not_far_ones() {
        let mut w = Foil::new();
        put(&mut w, 0, 0.0, 0.0, 32.0, 32.0, "rect", false);
        put(&mut w, 1, -20.0, 0.0, 26.0, 26.0, "rect", false);
        assert_eq!(w.pose(1).unwrap().0, -20.0);
        put(&mut w, 2, 0.0, 0.0, 8.0, 8.0, "circle", true);
        for i in 0..200 {
            let x = 400.0 + (i % 20) as f32 * 32.0;
            let y = -300.0 + (i / 20) as f32 * 32.0;
            put(&mut w, 10 + i, x, y, 32.0, 32.0, "rect", false);
        }
        let hits = w.overlaps(2);
        assert!(hits.contains(&0), "bullet should hit the near wall, got {hits:?}");
        assert!(hits.iter().all(|i| *i < 10), "far walls leaked into {hits:?}");
        assert!(w.overlaps(1).contains(&0));
    }

    #[test]
    fn circle_rect_and_negative_cells() {
        let mut w = Foil::new();
        put(&mut w, 3, -200.0, -180.0, 32.0, 32.0, "rect", false);
        put(&mut w, 4, -200.0, -180.0, 8.0, 8.0, "circle", true);
        assert_eq!(w.overlaps(4), vec![3]);
        w.set_pose(4, -160.0, -180.0);
        assert!(w.overlaps(4).is_empty(), "{:?}", w.overlaps(4));
    }

    #[test]
    fn sensor_query_skips_other_sensors_solid_still_sees_them() {
        let mut w = Foil::new();
        put(&mut w, 0, 0.0, 0.0, 32.0, 32.0, "rect", false);
        put(&mut w, 1, 0.0, 0.0, 8.0, 8.0, "circle", true);
        put(&mut w, 2, 1.0, 0.0, 8.0, 8.0, "circle", true);
        put(&mut w, 3, 0.0, 0.0, 26.0, 26.0, "rect", false);
        let hits = w.overlaps(1);
        assert!(hits.contains(&0), "bullet still hits the wall, got {hits:?}");
        assert!(!hits.contains(&2), "bullet must not test the other bullet, got {hits:?}");
        let hits = w.overlaps(3);
        assert!(hits.contains(&1) && hits.contains(&2), "solid must still see sensors, got {hits:?}");
        let pairs = w.query(&[1, 2, 3]);
        assert!(pairs.contains(&(1, 0)), "batch sensor hits the wall, got {pairs:?}");
        assert!(!pairs.iter().any(|&(a, b)| a == 1 && b == 2), "batch must skip sensor-sensor, got {pairs:?}");
        assert!(pairs.contains(&(3, 1)) && pairs.contains(&(3, 2)), "batch solid still sees sensors, got {pairs:?}");
        let mut sorted = pairs.clone();
        let n = sorted.len();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), n, "batch duplicated a pair");
    }

    #[test]
    fn batch_query_parallel_splits_without_duplicating() {
        let mut w = Foil::new();
        put(&mut w, 0, 0.0, 0.0, 32.0, 32.0, "rect", false);
        let mut idxs = Vec::new();
        for i in 0..400 {
            let x = if i < 200 { 0.0 } else { 4000.0 + i as f32 };
            put(&mut w, 1 + i, x, 0.0, 8.0, 8.0, "circle", true);
            idxs.push(1 + i);
        }
        let pairs = w.query(&idxs);
        let hits: Vec<_> = pairs.iter().filter(|(_, b)| *b == 0).map(|(a, _)| *a).collect();
        assert_eq!(hits.len(), 200, "near sensors should each hit the wall once, got {}", hits.len());
        assert!(hits.iter().all(|a| *a >= 1 && *a <= 200), "{hits:?}");
        assert!(pairs.iter().all(|(_, b)| *b == 0), "sensor-sensor leaked: {pairs:?}");
    }

    #[test]
    fn group_mask_keeps_both_directions() {
        let mut w = Foil::new();
        w.add(0, 0.0, 0.0, 32.0, 32.0, "rect", false, 0, 0).unwrap();
        w.add(1, 0.0, 0.0, 26.0, 26.0, "rect", false, 1, 0).unwrap();
        w.add(2, 0.0, 0.0, 26.0, 26.0, "rect", false, 2, 0).unwrap();
        w.add(3, 0.0, 0.0, 8.0, 8.0, "circle", true, 4, 2).unwrap();
        w.add(4, 0.0, 0.0, 8.0, 8.0, "circle", true, 8, 1).unwrap();
        let pairs = w.query(&[1, 2, 3, 4]);
        assert!(pairs.contains(&(3, 0)), "mask still hits group 0, got {pairs:?}");
        assert!(pairs.contains(&(3, 2)), "player bullet hits the enemy, got {pairs:?}");
        assert!(!pairs.iter().any(|&(a, b)| a == 3 && b == 1), "player bullet hit the player: {pairs:?}");
        assert!(pairs.contains(&(4, 1)) && pairs.contains(&(1, 4)), "both directions, got {pairs:?}");
        assert!(!pairs.iter().any(|&(a, b)| a == 4 && b == 2), "enemy bullet hit an enemy: {pairs:?}");
        assert!(pairs.contains(&(2, 3)), "enemy still sees the player bullet, got {pairs:?}");
    }

    #[test]
    fn unknown_name_is_rejected_and_len_skips_sensors() {
        let mut w = Foil::new();
        assert_eq!(w.add(0, 0.0, 0.0, 8.0, 8.0, "box", false, 0, 0), Err(SpecError::Shape));
        assert!(!w.contains(0));
        w.add(0, 0.0, 0.0, 10.0, 20.0, "rect", false, 3, 0).unwrap();
        w.add(1, 0.0, 0.0, 8.0, 8.0, "circle", true, 1, 4).unwrap();
        assert_eq!(w.len(), 1);
        assert_eq!(w.count(), 2);
        assert_eq!(w.size(0), Some((10.0, 20.0)));
        assert_eq!(w.group(1), Some(1));
        assert_eq!(w.mask(1), Some(4));
        w.set_sensor(1, false);
        assert!(!w.is_sensor(1));
        assert_eq!(w.len(), 2);
        let mut saw = false;
        w.each(|b| {
            if b.id == 1 {
                saw = !b.is_sensor;
            }
        });
        assert!(saw);
    }

    #[test]
    fn max_id_zero_size_and_f64() {
        let mut w = Foil::new();
        let id = u32::MAX as usize;
        w.add(id, 0.0, 0.0, 10.0, 10.0, "rect", false, 0, 0).unwrap();
        w.add(1, 0.0, 0.0, 10.0, 10.0, "rect", false, 0, 0).unwrap();
        w.add(2, 0.0, 0.0, 0.0, 0.0, "rect", false, 0, 0).unwrap();
        assert_eq!(w.size(2), Some((0.0, 0.0)));
        assert!(w.overlaps(id).contains(&1));
        assert!(w.query(&[id]).iter().any(|&(a, b)| a == id && b == 1));

        let mut wide = Foil::open_with("cpu", "f64").unwrap();
        wide.add(1, 16_777_216.0, 0.0, 1.0, 1.0, "rect", false, 0, 0).unwrap();
        wide.add(2, 16_777_217.0, 0.0, 1.0, 1.0, "rect", false, 0, 0).unwrap();
        assert_eq!(wide.pose(2).unwrap().0, 16_777_217.0);
        assert!(!wide.overlaps(1).contains(&2));
        assert!(Foil::open_with("dx12", "f64").is_err());
    }

    #[test]
    fn move_by_slides_on_contact_and_lets_sensors_through() {
        let mut w = Foil::new();
        put(&mut w, 0, 100.0, 0.0, 32.0, 32.0, "rect", false);
        put(&mut w, 1, 0.0, 0.0, 20.0, 20.0, "rect", false);
        let (x, y) = w.move_by(1, 200.0, 0.0);
        assert_eq!((x, y), (74.0, 0.0));
        assert!(w.overlaps(1).is_empty(), "{:?}", w.overlaps(1));

        let (x, y) = w.move_by(1, 30.0, 80.0);
        assert_eq!((x, y), (74.0, 80.0), "贴着墙时竖直还能走");

        put(&mut w, 2, 74.0, 200.0, 20.0, 32.0, "rect", false);
        let (x, y) = w.move_by(1, 0.0, 200.0);
        assert_eq!((x, y), (74.0, 174.0));
        assert!(w.overlaps(1).is_empty(), "{:?}", w.overlaps(1));

        put(&mut w, 3, 0.0, 40.0, 10.0, 10.0, "circle", true);
        let (x, y) = w.move_by(3, 100.0, -40.0);
        assert_eq!((x, y), (100.0, 0.0));
        assert!(w.overlaps(3).contains(&0));

        assert_eq!(w.move_by(9, 1.0, 1.0), (0.0, 0.0));
    }

    #[test]
    fn move_by_circle_stops_on_the_touching_line() {
        let mut w = Foil::new();
        put(&mut w, 0, 40.0, 0.0, 20.0, 20.0, "rect", false);
        put(&mut w, 1, 0.0, 0.0, 10.0, 10.0, "circle", false);
        let (x, y) = w.move_by(1, 100.0, 0.0);
        assert_eq!((x, y), (25.0, 0.0));
        assert!(w.overlaps(1).is_empty(), "{:?}", w.overlaps(1));
    }

    #[test]
    fn move_by_stops_on_the_wall() {
        let mut w = Foil::new();
        put(&mut w, 0, 100.0, 0.0, 32.0, 32.0, "rect", false);
        put(&mut w, 1, 0.0, 0.0, 20.0, 20.0, "rect", false);
        put(&mut w, 2, -400.0, -400.0, 16.0, 16.0, "rect", false);
        put(&mut w, 3, -400.0, -400.0, 8.0, 8.0, "circle", true);
        let (x, y) = w.move_by(1, 200.0, 0.0);
        assert_eq!((x, y), (74.0, 0.0));
        assert!(w.overlaps(1).is_empty(), "{:?}", w.overlaps(1));
        assert!(w.overlaps(3).contains(&2), "{:?}", w.overlaps(3));
    }

    #[test]
    fn move_by_pushes_out_of_an_existing_overlap() {
        let mut w = Foil::new();
        put(&mut w, 1, 0.0, 0.0, 6.0, 6.0, "rect", false);
        put(&mut w, 2, 4.0, 0.0, 6.0, 6.0, "rect", false);
        let (x, y) = w.move_by(1, 0.0, 0.0);
        assert_eq!((x, y), (-2.0, 0.0));
        assert!(w.overlaps(1).is_empty(), "{:?}", w.overlaps(1));
    }

    #[test]
    fn move_by_head_on_keeps_list_order() {
        let mut w = Foil::new();
        put(&mut w, 1, 0.0, 0.0, 6.0, 6.0, "rect", false);
        put(&mut w, 2, 20.0, 0.0, 6.0, 6.0, "rect", false);
        let a = w.move_by(1, 30.0, 0.0);
        let b = w.move_by(2, -30.0, 0.0);
        assert_eq!(a, (14.0, 0.0));
        assert_eq!(b, (20.0, 0.0));
        assert!(w.overlaps(1).is_empty(), "{:?}", w.overlaps(1));
        assert!(w.overlaps(2).is_empty(), "{:?}", w.overlaps(2));
    }
