//! Drawing a screen through its transition (UI-16): the sprites the screen
//! pushed are faded, moved or scaled after the fact, so widgets need not know.

use orbit_render2d::Scene;
use orbit_ui_data::{ScreenFx, ScreenTransition};

/// Applies `fx` to the sprites of `scene` from index `from` on (what one
/// screen drew), scaling around the center of a `canvas`-sized canvas.
pub fn apply_screen_fx(scene: &mut Scene, from: usize, fx: ScreenFx, canvas: [f32; 2]) {
    if fx.is_none() {
        return;
    }
    let center = [canvas[0] * 0.5, canvas[1] * 0.5];
    let map = |p: [f32; 2]| {
        [
            center[0] + (p[0] - center[0]) * fx.scale + fx.offset[0],
            center[1] + (p[1] - center[1]) * fx.scale + fx.offset[1],
        ]
    };
    let alpha = if fx.alpha.is_finite() { fx.alpha.clamp(0.0, 1.0) } else { 1.0 };
    for s in scene.sprites_mut().iter_mut().skip(from) {
        s.tint[3] *= alpha;
        s.position = map(s.position);
        s.scale = [s.scale[0] * fx.scale, s.scale[1] * fx.scale];
        if let Some(c) = &mut s.clip {
            let at = map([c[0], c[1]]);
            *c = [at[0], at[1], c[2] * fx.scale, c[3] * fx.scale];
        }
    }
}

/// How far a screen transition is: `elapsed` seconds into `t`, eased with
/// its curve (`named` resolves `Curve::Named`), as the share shown (`opening`)
/// or still shown (closing). `None` once it is over.
pub fn transition_shown(
    t: &ScreenTransition,
    elapsed: f32,
    opening: bool,
    named: &dyn Fn(&str) -> Option<orbit_core::Curve>,
) -> Option<f32> {
    let total = t.seconds();
    if total <= 0.0 || !elapsed.is_finite() || elapsed >= total {
        return None;
    }
    let raw = (elapsed.max(0.0) / total).clamp(0.0, 1.0);
    let eased = match &t.ease {
        Some(curve) => {
            let owned: std::collections::BTreeMap<String, orbit_core::Curve> = match curve {
                orbit_core::Curve::Named(n) => named(n).map(|c| (n.clone(), c)).into_iter().collect(),
                _ => Default::default(),
            };
            curve.eval_with(raw, |n| owned.get(n))
        }
        None => raw,
    };
    Some(if opening { eased } else { 1.0 - eased })
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_render2d::{Sprite2d, TextureId};
    use orbit_ui_data::ScreenTransitionKind;

    #[test]
    fn fx_moves_fades_and_scales_what_was_drawn_after_from() {
        let mut scene = Scene::new();
        let tex = TextureId::placeholder();
        scene.push(Sprite2d::new(tex, [10.0, 10.0], [4.0, 4.0]));
        scene.push(Sprite2d::new(tex, [100.0, 50.0], [4.0, 4.0]).clip([0.0, 0.0, 200.0, 100.0]));
        let fx = ScreenFx {
            alpha: 0.5,
            offset: [0.0, 10.0],
            scale: 0.5,
        };
        apply_screen_fx(&mut scene, 1, fx, [200.0, 100.0]);
        let s = scene.sprites();
        assert_eq!(s[0].position, [10.0, 10.0], "before `from`: untouched");
        assert_eq!(s[1].position, [100.0, 60.0]);
        assert_eq!(s[1].tint[3], 0.5);
        assert_eq!(s[1].scale, [0.5, 0.5]);
        assert_eq!(s[1].clip, Some([50.0, 35.0, 100.0, 50.0]));
    }

    #[test]
    fn progress_ends_and_runs_backwards_when_closing() {
        let t = ScreenTransition {
            kind: ScreenTransitionKind::Fade,
            duration: 1.0,
            ease: None,
        };
        let none = |_: &str| None;
        assert_eq!(transition_shown(&t, 0.25, true, &none), Some(0.25));
        assert_eq!(transition_shown(&t, 0.25, false, &none), Some(0.75));
        assert_eq!(transition_shown(&t, 1.0, true, &none), None);
        let cut = ScreenTransition { duration: 0.0, ..t };
        assert_eq!(transition_shown(&cut, 0.0, true, &none), None);
    }
}
