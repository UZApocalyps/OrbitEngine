//! Rigged expressions on the stage (ADR 0013 §4, M5B-07): loading the
//! `.rig.ron` assets with the pictures they draw, and posing / drawing a
//! character whose expression plays a rig clip.
//!
//! Everything about time is derived from the animation clock
//! ([`crate::stage::AnimClock`]): the pose is a pure function of
//! `(expression, clip, loop mode, clock)`, nothing is stored per frame, so
//! saves, rollback and skip need no extra state (like frame animations, the
//! clock simply restarts with the game).

use std::sync::OnceLock;

use orbit_anim::rig::{CpuSkinner, PreparedRig, RigPose};
use orbit_assets::{AssetHandle, AssetLoader, AssetPoll, ImageData};
use orbit_core::{LoopMode, RiggedMotion, Story};
use orbit_render2d::{Layer, Renderer2d, RigPlacement, Scene, TextureOptions, push_rig};

use crate::loading::AssetStore;
use crate::stage::{
    AnimClock, StageCharacter, animate_placement, character_placement,
};

/// A `.rig.ron` being loaded, then the pictures it draws (read from the
/// project / pak by the paths the rig names).
pub(crate) struct RigLoad {
    loader: AssetLoader,
    rig: AssetHandle<PreparedRig>,
    images: OnceLock<Vec<(String, AssetHandle<ImageData>)>>,
}

impl RigLoad {
    pub(crate) fn start(loader: &AssetLoader, path: &str) -> Self {
        let rig = loader.load_with(path, |_, bytes| {
            let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
            let loaded = orbit_anim::rig::load_rig(&text).map_err(|e| e.to_string())?;
            PreparedRig::new(loaded.rig).map_err(|issues| {
                issues
                    .iter()
                    .map(|i| format!("{i:?}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            })
        });
        Self {
            loader: loader.clone(),
            rig,
            images: OnceLock::new(),
        }
    }

    fn images(&self) -> Option<&Vec<(String, AssetHandle<ImageData>)>> {
        let AssetPoll::Ready(rig) = self.rig.poll() else {
            return None;
        };
        Some(self.images.get_or_init(|| {
            let mut paths: Vec<&str> = Vec::new();
            for skin in &rig.rig().skins {
                for att in skin.attachments.values() {
                    if !paths.contains(&att.image()) {
                        paths.push(att.image());
                    }
                }
            }
            paths
                .into_iter()
                .map(|p| (p.to_owned(), self.loader.load_image(p)))
                .collect()
        }))
    }

    pub(crate) fn is_done(&self) -> bool {
        match self.rig.poll() {
            AssetPoll::Loading => false,
            AssetPoll::Failed(_) => true,
            AssetPoll::Ready(_) => self.images().is_some_and(|images| {
                images
                    .iter()
                    .all(|(_, h)| !matches!(h.poll(), AssetPoll::Loading))
            }),
        }
    }

    /// Stores the rig and uploads its pictures. Failures are returned (the
    /// character then shows its poster, or the parts that did load).
    pub(crate) fn finish(
        self,
        renderer: &mut Renderer2d,
        store: &mut AssetStore,
        path: &str,
    ) -> Vec<String> {
        let mut failures = Vec::new();
        let rig = match self.rig.poll() {
            AssetPoll::Ready(rig) => rig,
            AssetPoll::Failed(e) => return vec![format!("rig `{path}`: {e}")],
            AssetPoll::Loading => return vec![format!("rig `{path}` was not loaded")],
        };
        if let Some(images) = self.images() {
            for (image, handle) in images {
                if store.texture(image).is_some() {
                    continue;
                }
                match handle.poll() {
                    AssetPoll::Ready(img) => {
                        match renderer.create_texture(
                            image,
                            img.width,
                            img.height,
                            &img.rgba,
                            TextureOptions::COLOR,
                        ) {
                            Ok(id) => store.insert_rig_texture(image, id),
                            Err(e) => failures.push(format!("rig picture `{image}`: {e}")),
                        }
                    }
                    AssetPoll::Failed(e) => failures.push(format!("rig `{path}` draws `{image}`: {e}")),
                    AssetPoll::Loading => {}
                }
            }
        }
        store.insert_rig(path, rig);
        failures
    }
}

/// Time inside one pass of a clip of `duration` seconds, `loop_mode` applied
/// to `elapsed` (the loop mode of the expression overrides the clip's own).
pub fn clip_time(duration: f32, loop_mode: LoopMode, elapsed: f32) -> f32 {
    if !(duration.is_finite() && duration > 0.0) || !elapsed.is_finite() || elapsed <= 0.0 {
        return 0.0;
    }
    match loop_mode {
        LoopMode::Loop => elapsed % duration,
        LoopMode::Once => elapsed.min(duration),
        LoopMode::PingPong => {
            let p = elapsed % (2.0 * duration);
            if p <= duration { p } else { 2.0 * duration - p }
        }
    }
}

/// The pose of `rig` playing the clip of `motion` `elapsed` seconds in (the
/// setup pose when the clip does not exist).
pub fn clip_pose(
    rig: &PreparedRig,
    story: &Story,
    motion: &RiggedMotion,
    elapsed: f32,
) -> RigPose {
    let Some(clip) = rig.rig().clips.get(&motion.clip) else {
        return rig.setup_pose();
    };
    let local = clip_time(clip.duration, motion.loop_mode, elapsed);
    rig.sample_clip_local_with(clip, local, |name| story.curves.get(name))
}

/// The pose of a character on stage at the clock's time, with the
/// cross-fade from the expression it just left when both play clips of the
/// same rig and skin.
pub fn stage_pose(
    rig: &PreparedRig,
    story: &Story,
    c: &StageCharacter,
    motion: &RiggedMotion,
    clock: Option<&AnimClock>,
) -> RigPose {
    let elapsed = clock.map_or(0.0, |a| a.rig_elapsed(c.character, &c.expression));
    let now = clip_pose(rig, story, motion, elapsed);
    let Some(fade) = clock.and_then(|a| a.rig_fade(c.character, &c.expression)) else {
        return now;
    };
    let previous = story
        .character(c.character)
        .and_then(|ch| ch.expression(&fade.previous))
        .and_then(|e| e.rigged())
        .filter(|p| {
            // No skin = the default one: `None` and `Some("default")` match.
            let skin = |m: &RiggedMotion| m.skin.clone().unwrap_or_else(|| "default".into());
            p.rig == motion.rig && skin(p) == skin(motion)
        });
    match previous {
        Some(prev) => {
            let before = clip_pose(rig, story, prev, fade.previous_elapsed);
            RigPose::blend(&before, &now, fade.weight)
        }
        None => now,
    }
}

/// Draws character `c` when its expression plays a rig clip and the rig is
/// loaded. `false`: nothing was drawn, the caller draws the poster.
#[allow(clippy::too_many_arguments)]
pub(crate) fn push_rigged_character(
    renderer: &Renderer2d,
    store: &AssetStore,
    story: &Story,
    c: &StageCharacter,
    clock: Option<&AnimClock>,
    fx: &orbit_core::StageFx,
    z: f32,
    scene: &mut Scene,
) -> bool {
    let Some(expression) = story
        .character(c.character)
        .and_then(|ch| ch.expression(&c.expression))
    else {
        return false;
    };
    let Some(motion) = expression.rigged() else {
        return false;
    };
    let Some(rig) = story
        .assets
        .get(motion.rig)
        .and_then(|a| store.rig(&a.path))
    else {
        return false;
    };
    let Some(poster) = story
        .assets
        .get(expression.image)
        .and_then(|a| store.texture(&a.path))
        .and_then(|t| renderer.texture_size(t))
    else {
        return false;
    };
    if poster[0] == 0 || poster[1] == 0 {
        return false;
    }
    let pose = stage_pose(rig, story, c, motion, clock);
    let world = rig.world(&pose);
    let list = rig.draw_list(&pose, &world, motion.skin.as_deref(), &CpuSkinner);
    // Rig units are the poster's pixels, the origin is the character's
    // bottom-center anchor.
    let placement = character_placement(c, poster);
    let animated = animate_placement(&placement, c.character, fx);
    let k = placement.size[1] / poster[1] as f32;
    let scale = k * animated.scale[0];
    let aspect = if animated.scale[0] != 0.0 {
        animated.scale[1] / animated.scale[0]
    } else {
        1.0
    };
    let mut position = animated.anchor;
    if placement.flip_y {
        // Flipped in place: mirror around the middle of the picture.
        position[1] -= poster[1] as f32 * k * animated.scale[1];
    }
    let rig_placement = RigPlacement {
        position,
        scale,
        aspect,
        rotation: animated.rotation,
        flip_x: placement.flip_x,
        flip_y: placement.flip_y,
        layer: Layer::CHARACTERS,
        z,
        tint: animated.tint,
    };
    let pushed = push_rig(scene, &list, &rig_placement, |image| {
        store.texture(image)
    });
    pushed > 0 || list.items.is_empty()
}

/// Whether the rig and the pictures of a rigged expression are loaded.
pub fn rig_ready(store: &AssetStore, story: &Story, expression: &orbit_core::Expression) -> bool {
    expression
        .rigged()
        .and_then(|m| story.assets.get(m.rig))
        .is_some_and(|a| store.rig(&a.path).is_some())
}


#[cfg(test)]
mod fixture_tests;
#[cfg(test)]
mod tests;
