use crate::*;

fn all_eases() -> Vec<Ease> {
    use Ease::*;
    vec![
        Linear, InQuad, OutQuad, InOutQuad, InCubic, OutCubic, InOutCubic, InQuart, OutQuart,
        InOutQuart, InQuint, OutQuint, InOutQuint, InSine, OutSine, InOutSine, InExpo, OutExpo,
        InOutExpo, InCirc, OutCirc, InOutCirc, InBack, OutBack, InOutBack, InElastic, OutElastic,
        InOutElastic, InBounce, OutBounce, InOutBounce, Step,
    ]
}

fn sample_curves() -> Vec<Curve> {
    let mut v: Vec<Curve> = all_eases().into_iter().map(Curve::Preset).collect();
    v.push(Curve::css_ease());
    v.push(Curve::css_ease_in());
    v.push(Curve::css_ease_out());
    v.push(Curve::css_ease_in_out());
    v.push(Curve::bezier(0.0, 0.0, 1.0, 1.0));
    v.push(Curve::bezier(0.0, 0.0, 0.0, 0.0));
    v.push(Curve::bezier(1.0, 1.0, 1.0, 1.0));
    v.push(Curve::bezier(0.68, -0.6, 0.32, 1.6));
    v.push(Curve::bezier(5.0, -3.0, -2.0, 4.0)); // x out of range gets clamped
    v.push(Curve::Named("nope".into()));
    v
}

#[test]
fn endpoints_are_exact_for_every_curve() {
    for c in sample_curves() {
        assert_eq!(c.eval(0.0), 0.0, "{c:?}");
        assert_eq!(c.eval(1.0), 1.0, "{c:?}");
        assert_eq!(c.eval(-3.0), 0.0, "{c:?}");
        assert_eq!(c.eval(7.0), 1.0, "{c:?}");
        assert_eq!(c.eval(f32::NAN), 0.0, "{c:?}");
        assert_eq!(c.eval(f32::INFINITY), 1.0, "{c:?}");
    }
}

#[test]
fn all_values_are_finite() {
    for c in sample_curves() {
        for i in 0..=1000 {
            let v = c.eval(i as f32 / 1000.0);
            assert!(v.is_finite(), "{c:?} at {i}");
        }
    }
}

#[test]
fn monotonic_presets_do_not_decrease() {
    use Ease::*;
    for e in [
        Linear, InQuad, OutQuad, InOutQuad, InCubic, OutCubic, InOutCubic, InQuart, OutQuart,
        InOutQuart, InQuint, OutQuint, InOutQuint, InSine, OutSine, InOutSine, InExpo, OutExpo,
        InOutExpo, InCirc, OutCirc, InOutCirc, Step,
    ] {
        let mut prev = 0.0;
        for i in 0..=1000 {
            let v = e.eval(i as f32 / 1000.0);
            assert!(v >= prev - 1e-6, "{e:?} decreases at {i}: {prev} -> {v}");
            assert!((-1e-6..=1.0 + 1e-6).contains(&v), "{e:?} leaves [0,1]");
            prev = v;
        }
    }
}

#[test]
fn symmetry_of_in_out_pairs() {
    // out(t) = 1 - in(1 - t)
    let pairs = [
        (Ease::InQuad, Ease::OutQuad),
        (Ease::InCubic, Ease::OutCubic),
        (Ease::InSine, Ease::OutSine),
        (Ease::InBack, Ease::OutBack),
        (Ease::InBounce, Ease::OutBounce),
        (Ease::InElastic, Ease::OutElastic),
    ];
    for (i, o) in pairs {
        for k in 1..100 {
            let t = k as f32 / 100.0;
            assert!((o.eval(t) - (1.0 - i.eval(1.0 - t))).abs() < 1e-4, "{i:?} {t}");
        }
    }
}

#[test]
fn in_out_midpoint_is_half() {
    for e in [Ease::InOutQuad, Ease::InOutCubic, Ease::InOutSine, Ease::InOutBounce] {
        assert!((e.eval(0.5) - 0.5).abs() < 1e-5, "{e:?}");
    }
}

#[test]
fn linear_and_known_values() {
    assert_eq!(Ease::Linear.eval(0.25), 0.25);
    assert!((Ease::InQuad.eval(0.5) - 0.25).abs() < 1e-6);
    assert!((Ease::OutQuad.eval(0.5) - 0.75).abs() < 1e-6);
    assert!((Ease::InCubic.eval(0.5) - 0.125).abs() < 1e-6);
    assert_eq!(Ease::Step.eval(0.999), 0.0);
}

#[test]
fn back_overshoots_bounce_and_elastic_oscillate() {
    // InBack dips below 0, OutBack goes above 1.
    assert!((0..100).any(|i| Ease::InBack.eval(i as f32 / 100.0) < -0.01));
    assert!((0..100).any(|i| Ease::OutBack.eval(i as f32 / 100.0) > 1.01));
    // OutBounce touches 1 several times: count descents.
    let mut descents = 0;
    let mut prev = 0.0;
    for i in 1..=1000 {
        let v = Ease::OutBounce.eval(i as f32 / 1000.0);
        if v < prev - 1e-6 {
            descents += 1;
            prev = v;
            continue;
        }
        prev = v;
    }
    assert!(descents >= 3);
    // OutElastic overshoots 1.
    assert!((0..1000).any(|i| Ease::OutElastic.eval(i as f32 / 1000.0) > 1.05));
    // Bounce never goes outside [0,1].
    for i in 0..=1000 {
        let v = Ease::OutBounce.eval(i as f32 / 1000.0);
        assert!((0.0..=1.0 + 1e-6).contains(&v));
    }
}

#[test]
fn bezier_linear_is_identity() {
    let c = Curve::bezier(0.0, 0.0, 1.0, 1.0);
    for i in 0..=100 {
        let t = i as f32 / 100.0;
        assert!((c.eval(t) - t).abs() < 1e-4, "{t}");
    }
}

#[test]
fn bezier_css_reference_values() {
    // cubic-bezier(0.42, 0, 0.58, 1) is symmetric around the center.
    let c = Curve::css_ease_in_out();
    assert!((c.eval(0.5) - 0.5).abs() < 1e-4);
    let a = c.eval(0.25);
    let b = c.eval(0.75);
    assert!((a + b - 1.0).abs() < 1e-4);
    assert!(a < 0.25 && b > 0.75);
    // CSS `ease` at 0.5 is about 0.8024 (browser reference value).
    assert!((Curve::css_ease().eval(0.5) - 0.8024).abs() < 2e-3);
}

#[test]
fn bezier_ease_in_out_is_monotonic() {
    for c in [Curve::css_ease(), Curve::css_ease_in(), Curve::css_ease_out(), Curve::css_ease_in_out()] {
        let mut prev = 0.0;
        for i in 0..=1000 {
            let v = c.eval(i as f32 / 1000.0);
            assert!(v >= prev - 1e-5, "{c:?} at {i}");
            prev = v;
        }
    }
}

#[test]
fn bezier_overshoot_allowed_and_x_clamped() {
    let c = Curve::bezier(0.34, 1.56, 0.64, 1.0);
    assert!((0..100).any(|i| c.eval(i as f32 / 100.0) > 1.01));
    // x outside [0,1] is clamped, so it equals the clamped curve.
    let a = Curve::bezier(2.0, 0.5, -1.0, 0.5);
    let b = Curve::bezier(1.0, 0.5, 0.0, 0.5);
    for i in 0..=20 {
        let t = i as f32 / 20.0;
        assert_eq!(a.eval(t), b.eval(t));
    }
}

#[test]
fn bezier_degenerate_control_points_converge() {
    // Vertical tangents (x1 = x2 = 0 or 1) make Newton fail; bisection saves it.
    for c in [Curve::bezier(0.0, 0.0, 0.0, 1.0), Curve::bezier(1.0, 0.0, 1.0, 1.0), Curve::bezier(0.0, 1.0, 1.0, 0.0)] {
        for i in 0..=200 {
            let v = c.eval(i as f32 / 200.0);
            assert!(v.is_finite() && (-0.01..=1.01).contains(&v), "{c:?} {v}");
        }
    }
}

#[test]
fn named_curves_resolve_without_cycles() {
    let table = std::collections::BTreeMap::from([
        ("fast".to_string(), Curve::Preset(Ease::OutQuad)),
        ("alias".to_string(), Curve::Named("fast".into())),
    ]);
    let r = |n: &str| table.get(n);
    assert_eq!(Curve::Named("fast".into()).eval_with(0.5, r), Ease::OutQuad.eval(0.5));
    // Named -> Named is linear, unknown is linear.
    assert_eq!(Curve::Named("alias".into()).eval_with(0.3, r), 0.3);
    assert_eq!(Curve::Named("zzz".into()).eval_with(0.3, r), 0.3);
    assert_eq!(Curve::Named("fast".into()).eval(0.3), 0.3);
}

#[test]
fn lerp_basic_types() {
    assert_eq!(f32::lerp(&2.0, &4.0, 0.5), 3.0);
    assert_eq!(Vec2::lerp(&Vec2::new(0.0, 10.0), &Vec2::new(10.0, 20.0), 0.5), Vec2::new(5.0, 15.0));
    let c = Rgba::lerp(&Rgba::new(0.0, 0.0, 0.0, 0.0), &Rgba::new(1.0, 0.5, 0.0, 1.0), 0.5);
    assert_eq!(c, Rgba::new(0.5, 0.25, 0.0, 0.5));
}

#[test]
fn lerp_is_exact_at_ends() {
    let (a, b) = (0.1f32, 0.7f32);
    assert_eq!(f32::lerp(&a, &b, 0.0), a);
    assert_eq!(f32::lerp(&a, &b, 1.0), b);
    let big = 1.0e8f32;
    assert_eq!(f32::lerp(&big, &(big + 3.0), 1.0), big + 3.0);
}

#[test]
fn color_overshoot_is_clamped() {
    let c = Rgba::lerp(&Rgba::new(0.0, 0.0, 0.0, 1.0), &Rgba::new(1.0, 1.0, 1.0, 1.0), 1.3);
    assert_eq!(c, Rgba::new(1.0, 1.0, 1.0, 1.0));
    let c = Rgba::lerp(&Rgba::new(0.0, 0.0, 0.0, 1.0), &Rgba::new(1.0, 1.0, 1.0, 1.0), -0.3);
    assert_eq!(c.r, 0.0);
}

#[test]
fn angle_takes_shortest_arc() {
    let v = Angle::lerp(&Angle(350.0), &Angle(10.0), 0.5);
    assert!((v.0 - 360.0).abs() < 1e-4, "{v:?}");
    let v = Angle::lerp(&Angle(10.0), &Angle(350.0), 0.5);
    assert!((v.0 - 0.0).abs() < 1e-4, "{v:?}");
    let v = Angle::lerp(&Angle(0.0), &Angle(90.0), 0.5);
    assert!((v.0 - 45.0).abs() < 1e-4);
    assert_eq!(Angle::lerp(&Angle(350.0), &Angle(10.0), 1.0), Angle(10.0));
}

#[test]
fn step_holds_until_segment_end() {
    let a = Step("a".to_string());
    let b = Step("b".to_string());
    assert_eq!(Step::lerp(&a, &b, 0.99), a);
    assert_eq!(Step::lerp(&a, &b, 1.0), b);
}

#[test]
fn tween_samples_and_ends_exact() {
    let t = Tween::new(0.0f32, 100.0, 2.0, Curve::Preset(Ease::InQuad));
    assert_eq!(t.sample(-1.0), 0.0);
    assert_eq!(t.sample(0.0), 0.0);
    assert!((t.sample(1.0) - 25.0).abs() < 1e-4);
    assert_eq!(t.sample(2.0), 100.0);
    assert_eq!(t.sample(99.0), 100.0);
    // Zero duration jumps to the end.
    let z = Tween::new(1.0f32, 5.0, 0.0, Curve::LINEAR);
    assert_eq!(z.sample(0.0), 5.0);
}

fn scalar_track() -> Track<f32> {
    Track::new(vec![
        Key::new(1.0, 10.0),
        Key::with_ease(2.0, 20.0, Curve::Preset(Ease::Step)),
        Key::new(4.0, 40.0),
    ])
}

#[test]
fn track_clamps_before_and_after() {
    let t = scalar_track();
    assert_eq!(t.sample(0.0), Some(10.0));
    assert_eq!(t.sample(-5.0), Some(10.0));
    assert_eq!(t.sample(4.0), Some(40.0));
    assert_eq!(t.sample(100.0), Some(40.0));
    assert_eq!(t.sample(f32::NAN), Some(10.0));
}

#[test]
fn track_interpolates_with_per_key_ease() {
    let t = scalar_track();
    assert_eq!(t.sample(1.0), Some(10.0));
    assert_eq!(t.sample(1.5), Some(15.0));
    // Key 2's ease is Step: holds 20 until the next key.
    assert_eq!(t.sample(2.0), Some(20.0));
    assert_eq!(t.sample(3.9), Some(20.0));
    assert_eq!(t.sample(4.0), Some(40.0));
}

#[test]
fn track_empty_and_single_key() {
    assert_eq!(Track::<f32>::default().sample(1.0), None);
    let one = Track::new(vec![Key::new(3.0, 7.0f32)]);
    assert_eq!(one.sample(0.0), Some(7.0));
    assert_eq!(one.sample(9.0), Some(7.0));
}

#[test]
fn track_validate_and_normalize() {
    let mut t = Track::new(vec![Key::new(2.0, 2.0f32), Key::new(1.0, 1.0), Key::new(2.0, 9.0)]);
    assert_eq!(t.validate(), Err(TrackError::NotSorted(1)));
    t.normalize();
    assert_eq!(t.validate(), Ok(()));
    assert_eq!(t.keys.len(), 2);
    assert_eq!(t.sample(2.0), Some(9.0)); // later duplicate wins
    assert_eq!(Track::new(vec![Key::new(-1.0, 0.0f32)]).validate(), Err(TrackError::BadTime(0)));
    assert_eq!(Track::new(vec![Key::new(f32::NAN, 0.0f32)]).validate(), Err(TrackError::BadTime(0)));
    assert_eq!(Track::new(vec![Key::new(1.0, 0.0f32), Key::new(1.0, 1.0)]).validate(), Err(TrackError::NotSorted(1)));
}

#[test]
fn track_with_duplicate_times_does_not_divide_by_zero() {
    // Unsorted-but-duplicated input must never yield NaN.
    let t = Track::new(vec![Key::new(1.0, 0.0f32), Key::new(1.0, 5.0), Key::new(2.0, 6.0)]);
    for i in 0..=30 {
        let v = t.sample(i as f32 / 10.0).unwrap();
        assert!(v.is_finite());
    }
}

#[test]
fn named_curve_in_key_ease() {
    let t = Track::new(vec![
        Key::with_ease(0.0, 0.0f32, Curve::Named("q".into())),
        Key::new(1.0, 1.0),
    ]);
    let table = std::collections::BTreeMap::from([("q".to_string(), Curve::Preset(Ease::InQuad))]);
    assert!((t.sample_with(0.5, |n| table.get(n)).unwrap() - 0.25).abs() < 1e-6);
    assert_eq!(t.sample(0.5), Some(0.5));
}

#[test]
fn discrete_track_steps() {
    let t = Track::new(vec![Key::new(0.0, Step("a".to_string())), Key::new(1.0, Step("b".to_string()))]);
    assert_eq!(t.sample(0.99).unwrap().0, "a");
    assert_eq!(t.sample(1.0).unwrap().0, "b");
}

fn clip(looping: Loop) -> Clip {
    Clip {
        duration: 2.0,
        looping,
        tracks: vec![ClipTrack {
            channel: "x".into(),
            data: TrackData::Scalar(Track::new(vec![Key::new(0.0, 0.0), Key::new(2.0, 2.0)])),
        }],
    }
}

fn x(c: &Clip, t: f32) -> f32 {
    match c.sample(t)["x"] {
        Value::Scalar(v) => v,
        _ => panic!(),
    }
}

#[test]
fn clip_once() {
    let c = clip(Loop::Once);
    assert_eq!(x(&c, 0.0), 0.0);
    assert_eq!(x(&c, 1.0), 1.0);
    assert_eq!(x(&c, 2.0), 2.0);
    assert_eq!(x(&c, 50.0), 2.0);
    assert_eq!(x(&c, -1.0), 0.0);
    assert!(!c.is_finished(1.99));
    assert!(c.is_finished(2.0));
}

#[test]
fn clip_loop_n() {
    let c = clip(Loop::Loop(2));
    assert_eq!(x(&c, 1.0), 1.0);
    assert_eq!(x(&c, 3.0), 1.0);
    assert!(!c.is_finished(3.9));
    assert!(c.is_finished(4.0));
    assert_eq!(x(&c, 4.0), 2.0); // final state exactly
    assert_eq!(x(&c, 1000.0), 2.0);
    // Loop(0) is finished at once and shows the final state.
    let z = clip(Loop::Loop(0));
    assert!(z.is_finished(0.0));
    assert_eq!(x(&z, 0.0), 2.0);
}

#[test]
fn clip_forever_and_pingpong() {
    let f = clip(Loop::Forever);
    assert_eq!(f.total_duration(), None);
    assert!(!f.is_finished(1.0e9));
    assert_eq!(x(&f, 2.5), 0.5);
    let p = clip(Loop::PingPong);
    assert_eq!(x(&p, 1.0), 1.0);
    assert_eq!(x(&p, 3.0), 1.0); // coming back
    assert_eq!(x(&p, 2.0), 2.0);
    assert_eq!(x(&p, 4.0), 0.0);
}

#[test]
fn clip_zero_duration_is_final_state() {
    let mut c = clip(Loop::Once);
    c.duration = 0.0;
    assert_eq!(x(&c, 0.0), 0.0); // local time 0 = duration
    c.looping = Loop::Forever;
    assert_eq!(x(&c, 5.0), 0.0); // no division by zero / NaN
}

#[test]
fn clip_multiple_channels_and_types() {
    let c = Clip {
        duration: 1.0,
        looping: Loop::Once,
        tracks: vec![
            ClipTrack { channel: "offset".into(), data: TrackData::Vec2(Track::new(vec![Key::new(0.0, Vec2::new(0.0, 0.0)), Key::new(1.0, Vec2::new(10.0, -10.0))])) },
            ClipTrack { channel: "tint".into(), data: TrackData::Color(Track::new(vec![Key::new(0.0, Rgba::new(1.0, 1.0, 1.0, 0.0)), Key::new(1.0, Rgba::new(1.0, 0.0, 0.0, 1.0))])) },
            ClipTrack { channel: "rot".into(), data: TrackData::Angle(Track::new(vec![Key::new(0.0, Angle(350.0)), Key::new(1.0, Angle(10.0))])) },
            ClipTrack { channel: "img".into(), data: TrackData::Name(Track::new(vec![Key::new(0.0, Step("a".into())), Key::new(0.5, Step("b".into()))])) },
            ClipTrack { channel: "empty".into(), data: TrackData::Scalar(Track::default()) },
        ],
    };
    let p = c.sample(0.5);
    assert_eq!(p["offset"], Value::Vec2(Vec2::new(5.0, -5.0)));
    assert_eq!(p["tint"], Value::Color(Rgba::new(1.0, 0.5, 0.5, 0.5)));
    assert_eq!(p["img"], Value::Name("b".into()));
    assert!(!p.contains_key("empty"));
    let Value::Angle(a) = p["rot"] else { panic!() };
    assert!((a.0 - 360.0).abs() < 1e-4);
    // Skip -> exact final state.
    let end = c.sample(c.duration);
    assert_eq!(end["offset"], Value::Vec2(Vec2::new(10.0, -10.0)));
    assert_eq!(end["img"], Value::Name("b".into()));
    assert!(c.tracks[0].data.end_time() == 1.0 && !c.tracks[0].data.is_empty());
}

#[test]
fn clip_sampling_is_deterministic() {
    let c = clip(Loop::PingPong);
    let a: Vec<f32> = (0..500).map(|i| x(&c, i as f32 * 0.037)).collect();
    let b: Vec<f32> = (0..500).map(|i| x(&c, i as f32 * 0.037)).collect();
    assert_eq!(a, b);
}

#[test]
fn clip_named_curve_resolution() {
    let mut c = clip(Loop::Once);
    if let TrackData::Scalar(t) = &mut c.tracks[0].data {
        t.keys[0].ease = Curve::Named("q".into());
    }
    let table = std::collections::BTreeMap::from([("q".to_string(), Curve::Preset(Ease::InQuad))]);
    let p = c.sample_with(1.0, |n| table.get(n));
    assert_eq!(p["x"], Value::Scalar(0.5)); // InQuad(0.5)=0.25 of 2.0
}

#[test]
fn serde_ron_roundtrip() {
    let mut c = clip(Loop::Loop(3));
    if let TrackData::Scalar(t) = &mut c.tracks[0].data {
        t.keys[0].ease = Curve::bezier(0.2, 0.0, 0.4, 1.4);
        t.keys[1].ease = Curve::Named("pop".into());
    }
    c.tracks.push(ClipTrack { channel: "e".into(), data: TrackData::Scalar(Track::new(vec![Key::with_ease(0.0, 1.0, Curve::Preset(Ease::OutBounce))])) });
    let s = ron::to_string(&c).unwrap();
    let back: Clip = ron::from_str(&s).unwrap();
    assert_eq!(back, c);
}

#[test]
fn serde_defaults_and_stable_format() {
    // `looping` and `ease` are optional in the file.
    let c: Clip = ron::from_str("(duration: 1.0, tracks: [(channel: \"a\", data: Scalar([(t: 0.0, value: 1.0), (t: 1.0, value: 2.0)]))])").unwrap();
    assert_eq!(c.looping, Loop::Once);
    assert_eq!(x_of(&c, "a", 0.5), 1.5);
    // Linear ease is omitted on output.
    let s = ron::to_string(&Key::new(0.0, 1.0f32)).unwrap();
    assert!(!s.contains("ease"), "{s}");
    let k: Curve = ron::from_str("Bezier(p1: (0.25, 0.1), p2: (0.25, 1.0))").unwrap();
    assert_eq!(k, Curve::css_ease());
    let k: Curve = ron::from_str("Preset(OutBack)").unwrap();
    assert_eq!(k, Curve::Preset(Ease::OutBack));
}

fn x_of(c: &Clip, ch: &str, t: f32) -> f32 {
    match c.sample(t)[ch] {
        Value::Scalar(v) => v,
        _ => panic!(),
    }
}
