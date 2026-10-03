//! Inspectors of the parallax background (`BackgroundLayers`) and of the
//! particle nodes (`Particles`, `StopParticles`), M5A-07 / M5A-08: the layer
//! list with depth and a pan preview, the emitter form with presets and a
//! live preview drawn by the very same pure sampler the game uses.

use egui::{Color32, ComboBox, DragValue, Pos2, Rect, RichText, Shape, Stroke, Ui, Vec2};
use orbit_core::{
    AssetKind, BgLayer, Emitter, MAX_BG_LAYERS, MAX_PARTICLES_PER_EMITTER, NodeId, NodeKind,
    ParticlePreset, Particle, Transition,
};
use orbit_editor_api::fields::{self, asset_picker, checkbox, color_field, transition_editor};
use orbit_editor_api::{EditorContext, UiText, icons};

use super::inspector::{commit, curves_of, info, tfield};
use super::thumbs;
use super::ui_util::weak;

fn drag(ui: &mut Ui, value: &mut f32, speed: f64, range: std::ops::RangeInclusive<f32>) -> bool {
    ui.add(
        DragValue::new(value)
            .speed(speed)
            .range(range)
            .max_decimals(3),
    )
    .changed()
}

fn pair(ui: &mut Ui, v: &mut [f32; 2], speed: f64, range: std::ops::RangeInclusive<f32>) -> bool {
    ui.horizontal(|ui| {
        let a = drag(ui, &mut v[0], speed, range.clone());
        let b = drag(ui, &mut v[1], speed, range.clone());
        a | b
    })
    .inner
}

// ------------------------------------------------------------ parallax

pub(super) fn layers(
    ui: &mut Ui,
    ctx: &mut EditorContext<'_>,
    id: NodeId,
    layers: &[BgLayer],
    transition: Option<Transition>,
) {
    let Some(story) = ctx.story() else { return };
    let first_image = story
        .assets
        .assets
        .iter()
        .find(|a| a.kind == AssetKind::Image)
        .map(|a| a.id);
    let mut edited: Vec<BgLayer> = layers.to_vec();
    let mut changed = false;
    let mut structural = false;
    let count = edited.len();
    let mut swap: Option<(usize, usize)> = None;
    let mut remove: Option<usize> = None;

    tfield(ui, "inspector.layers.list", |ui| {
        for (i, layer) in edited.iter_mut().enumerate() {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    thumbs::show(ui, ctx, story, layer.asset, egui::vec2(64.0, 36.0));
                    ui.vertical(|ui| {
                        if let Some(a) = asset_picker(ui, ("layer_asset", id, i), story, AssetKind::Image, layer.asset) {
                            layer.asset = a;
                            changed = true;
                        }
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(ui.t("inspector.layers.depth")).small().color(weak(ui)))
                                .on_hover_text(ui.t("inspector.layers.depth.tip"));
                            changed |= drag(ui, &mut layer.depth, 0.01, -4.0..=4.0);
                            if let Some(v) = checkbox(ui, layer.repeat_x, &ui.t("inspector.layers.repeat")) {
                                layer.repeat_x = v;
                                changed = true;
                            }
                        });
                    });
                });
                ui.horizontal(|ui| {
                    if i > 0
                        && ui.small_button(icons::ARROW_UP).on_hover_text(ui.t("inspector.layers.up")).clicked()
                    {
                        swap = Some((i, i - 1));
                    }
                    if i + 1 < count
                        && ui.small_button(icons::ARROW_DOWN).on_hover_text(ui.t("inspector.layers.down")).clicked()
                    {
                        swap = Some((i, i + 1));
                    }
                    if count > 1
                        && ui.small_button(icons::TRASH).on_hover_text(ui.t("inspector.layers.remove")).clicked()
                    {
                        remove = Some(i);
                    }
                });
            });
        }
        if count < MAX_BG_LAYERS
            && ui
                .button(format!("{}  {}", icons::PLUS, ui.t("inspector.layers.add")))
                .clicked()
            && let Some(asset) = first_image
        {
            let depth = edited.last().map_or(0.0, |l| l.depth + 0.2);
            edited.push(BgLayer::new(asset, depth));
            changed = true;
            structural = true;
        }
    });
    if let Some((a, b)) = swap {
        edited.swap(a, b);
        changed = true;
        structural = true;
    }
    if let Some(i) = remove {
        edited.remove(i);
        changed = true;
        structural = true;
    }
    if changed {
        commit(
            ctx,
            id,
            NodeKind::BackgroundLayers { layers: edited.clone(), transition: transition.clone() },
            if structural { "layers_structure" } else { "layers" },
        );
    }

    tfield(ui, "inspector.transition", |ui| {
        if let Some(t) = transition_editor(ui, ("layers_transition", id), transition.clone(), &curves_of(ctx)) {
            commit(
                ctx,
                id,
                NodeKind::BackgroundLayers { layers: edited.clone(), transition: t },
                "transition",
            );
        }
    });
    pan_preview(ui, ctx, id, &edited);
    info(ui, "inspector.layers.info");
}

/// A small panorama: the layers shifted by `pan x depth`, to see the
/// parallax without playing.
fn pan_preview(ui: &mut Ui, ctx: &EditorContext<'_>, id: NodeId, layers: &[BgLayer]) {
    let Some(story) = ctx.story() else { return };
    let Some(root) = thumbs::assets_root(ctx, story) else { return };
    let waker = ctx.state().waker.clone();
    let key = egui::Id::new(("layers_pan", id));
    let mut pan: f32 = ui.data(|d| d.get_temp(key)).unwrap_or(0.0);
    tfield(ui, "inspector.layers.preview", |ui| {
        let width = ui.available_width().min(320.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, width * 9.0 / 16.0), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, Color32::BLACK);
        for layer in layers {
            let Some(texture) = thumbs::thumbnail(ui.ctx(), &waker, &root, story, layer.asset) else {
                continue;
            };
            let tex = texture.size_vec2();
            if tex.x <= 0.0 || tex.y <= 0.0 {
                continue;
            }
            let scale = (rect.width() / tex.x).max(rect.height() / tex.y);
            let size = tex * scale;
            let center = rect.center() + Vec2::new(pan * layer.depth * rect.width(), 0.0);
            let centers: Vec<f32> = if layer.repeat_x {
                let wrapped = center.x - ((center.x - rect.center().x) / size.x).round() * size.x;
                vec![wrapped - size.x, wrapped, wrapped + size.x]
            } else {
                vec![center.x]
            };
            for x in centers {
                painter.image(
                    texture.id(),
                    Rect::from_center_size(Pos2::new(x, center.y), size),
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        }
        ui.spacing_mut().slider_width = width;
        ui.add(egui::Slider::new(&mut pan, -0.25..=0.25).show_value(false));
    });
    ui.data_mut(|d| d.insert_temp(key, pan));
}

// ----------------------------------------------------------- particles

fn preset_label(ui: &Ui, p: ParticlePreset) -> String {
    ui.t(&format!("inspector.particles.preset.{}", p.name()))
}

pub(super) fn particles(ui: &mut Ui, ctx: &mut EditorContext<'_>, id: NodeId, emitter: &Emitter) {
    let Some(story) = ctx.story() else { return };
    let mut e = emitter.clone();
    let mut changed = false;

    tfield(ui, "inspector.particles.preset", |ui| {
        let mut picked = None;
        ComboBox::from_id_salt(("particle_preset", id))
            .selected_text(preset_label(ui, e.preset))
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for p in ParticlePreset::ALL {
                    if ui.selectable_label(e.preset == p, preset_label(ui, p)).clicked() {
                        picked = Some(p);
                    }
                }
            });
        if let Some(p) = picked.filter(|p| *p != e.preset) {
            // A preset replaces every setting, keeping the identity.
            let (name, seed, sprite) = (e.name.clone(), e.seed, e.sprite);
            e = Emitter::preset(p);
            e.name = if name.is_empty() { p.name().to_owned() } else { name };
            e.seed = seed;
            e.sprite = sprite;
            changed = true;
        }
    });
    ui.label(RichText::new(ui.t("inspector.particles.preset.tip")).small().color(weak(ui)));
    tfield(ui, "inspector.particles.name", |ui| {
        let mut done = false;
        let hint = ui.t("inspector.particles.name_hint");
        if let Some(t) = fields::text_field(ui, ("particle_name", id), &e.name, &hint, 1, &mut done) {
            e.name = t;
            changed = true;
        }
    });
    preview(ui, &e);
    if let Some(problem) = e.problem() {
        ui.label(RichText::new(problem).small().color(ui.visuals().error_fg_color));
    } else {
        ui.label(
            RichText::new(ui.tf(
                "inspector.particles.count",
                &[("count", &(e.peak_count().ceil() as u32)), ("limit", &MAX_PARTICLES_PER_EMITTER)],
            ))
            .small()
            .color(weak(ui)),
        );
    }

    let mut seed = e.seed as f32;
    tfield(ui, "inspector.particles.seed", |ui| {
        if drag(ui, &mut seed, 1.0, 0.0..=1.0e9) {
            e.seed = seed as u32;
            changed = true;
        }
    });
    tfield(ui, "inspector.particles.rate", |ui| changed |= drag(ui, &mut e.rate, 0.5, 0.0..=2000.0));
    tfield(ui, "inspector.particles.lifetime", |ui| changed |= drag(ui, &mut e.lifetime, 0.05, 0.05..=60.0));
    tfield(ui, "inspector.particles.jitter", |ui| {
        changed |= drag(ui, &mut e.lifetime_jitter, 0.01, 0.0..=1.0);
    });
    tfield(ui, "inspector.particles.spawn", |ui| {
        ui.horizontal(|ui| {
            for v in e.spawn.iter_mut() {
                changed |= drag(ui, v, 0.005, -2.0..=3.0);
            }
        });
    });
    tfield(ui, "inspector.particles.direction", |ui| changed |= drag(ui, &mut e.direction, 1.0, -360.0..=720.0));
    tfield(ui, "inspector.particles.spread", |ui| changed |= drag(ui, &mut e.spread, 1.0, 0.0..=360.0));
    tfield(ui, "inspector.particles.speed", |ui| changed |= pair(ui, &mut e.speed, 0.005, 0.0..=5.0));
    tfield(ui, "inspector.particles.gravity", |ui| changed |= pair(ui, &mut e.gravity, 0.01, -5.0..=5.0));
    tfield(ui, "inspector.particles.sway", |ui| changed |= pair(ui, &mut e.sway, 0.005, 0.0..=2.0));
    tfield(ui, "inspector.particles.size", |ui| changed |= pair(ui, &mut e.size, 0.0005, 0.0005..=0.2));
    tfield(ui, "inspector.particles.stretch", |ui| changed |= drag(ui, &mut e.stretch, 0.05, 0.1..=30.0));
    tfield(ui, "inspector.particles.spin", |ui| changed |= drag(ui, &mut e.spin, 1.0, 0.0..=1440.0));
    tfield(ui, "inspector.particles.color", |ui| {
        ui.horizontal(|ui| {
            if let Some(c) = color_field(ui, e.color) {
                e.color = c;
                changed = true;
            }
            ui.label(icons::ARROW_RIGHT);
            if let Some(c) = color_field(ui, e.color_end) {
                e.color_end = c;
                changed = true;
            }
        });
    });
    tfield(ui, "inspector.particles.opacity", |ui| changed |= drag(ui, &mut e.opacity, 0.01, 0.0..=1.0));
    tfield(ui, "inspector.particles.fade", |ui| changed |= drag(ui, &mut e.fade, 0.01, 0.0..=0.5));

    tfield(ui, "inspector.particles.sprite", |ui| {
        let first = story
            .assets
            .assets
            .iter()
            .find(|a| a.kind == AssetKind::Image)
            .map(|a| a.id);
        let mut use_sprite = e.sprite.is_some();
        if let Some(v) = checkbox(ui, use_sprite, &ui.t("inspector.particles.sprite.use")) {
            use_sprite = v;
            e.sprite = if v { first } else { None };
            changed = true;
        }
        if let (true, Some(current)) = (use_sprite, e.sprite)
            && let Some(a) = asset_picker(ui, ("particle_sprite", id), story, AssetKind::Image, current)
        {
            e.sprite = Some(a);
            changed = true;
        }
    });
    ui.add_space(4.0);
    if let Some(v) = checkbox(ui, e.front, &ui.t("inspector.particles.front")) {
        e.front = v;
        changed = true;
    }
    if let Some(v) = checkbox(ui, e.prewarm, &ui.t("inspector.particles.prewarm")) {
        e.prewarm = v;
        changed = true;
    }
    if changed {
        commit(ctx, id, NodeKind::Particles(e), "emitter");
    }
    info(ui, "inspector.particles.info");
}

/// Live preview of the emitter in a 16:9 box: the game's own sampler, run on
/// the clock of the editor (looping, so a cold start is seen too).
fn preview(ui: &mut Ui, e: &Emitter) {
    let width = ui.available_width().min(320.0).max(120.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, width * 9.0 / 16.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, Color32::from_rgb(28, 32, 42));
    let cycle = (e.max_lifetime() * 1.5).clamp(3.0, 20.0);
    let t = ui.input(|i| i.time) as f32 % cycle;
    let mut live: Vec<Particle> = Vec::new();
    e.sample(t, None, &mut live);
    live.truncate(900);
    for p in &live {
        let center = Pos2::new(rect.left() + p.pos[0] * rect.width(), rect.top() + p.pos[1] * rect.height());
        let w = (p.size * rect.height()).max(1.0);
        let c = Color32::from_rgba_unmultiplied(
            (p.color[0] * 255.0) as u8,
            (p.color[1] * 255.0) as u8,
            (p.color[2] * 255.0) as u8,
            (p.color[3] * 255.0) as u8,
        );
        if p.stretch > 1.0 || p.rotation != 0.0 {
            let (sin, cos) = p.rotation.to_radians().sin_cos();
            let (hw, hh) = (w * 0.5, w * p.stretch * 0.5);
            let corner = |x: f32, y: f32| Pos2::new(center.x + x * cos - y * sin, center.y + x * sin + y * cos);
            painter.add(Shape::convex_polygon(
                vec![corner(-hw, -hh), corner(hw, -hh), corner(hw, hh), corner(-hw, hh)],
                c,
                Stroke::NONE,
            ));
        } else {
            painter.circle_filled(center, w * 0.5, c);
        }
    }
    ui.ctx().request_repaint();
}

pub(super) fn stop_particles(ui: &mut Ui, ctx: &mut EditorContext<'_>, id: NodeId, name: &str) {
    let Some(story) = ctx.story() else { return };
    let mut names: Vec<String> = story
        .scenes
        .iter()
        .flat_map(|s| &s.nodes)
        .filter_map(|n| match &n.kind {
            NodeKind::Particles(e) => Some(e.name.clone()),
            _ => None,
        })
        .collect();
    names.sort();
    names.dedup();
    let mut chosen = name.to_owned();
    tfield(ui, "inspector.stop_particles.name", |ui| {
        let all = ui.t("inspector.stop_particles.all");
        ComboBox::from_id_salt(("stop_particles", id))
            .selected_text(if name.is_empty() { all.clone() } else { name.to_owned() })
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut chosen, String::new(), all);
                for n in &names {
                    ui.selectable_value(&mut chosen, n.clone(), n);
                }
            });
    });
    if chosen != name {
        commit(ctx, id, NodeKind::StopParticles { name: chosen }, "name");
    }
    info(ui, "inspector.stop_particles.info");
}
