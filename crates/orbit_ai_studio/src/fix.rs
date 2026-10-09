//! Deterministic repair of the references a small model gets wrong in the
//! bible (undefined scenes, near-miss ids...), so its output is never thrown
//! away for something fixable.

use std::collections::HashSet;

use crate::bible::{Bible, LocationSpec, SceneOutline};

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

/// `Keeping Secret!` -> `keeping_secret` (empty when nothing usable).
fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    let out = out.trim_end_matches('_').to_owned();
    if out.is_empty() {
        return out;
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        return format!("s_{out}");
    }
    if orbit_dsl::printer::KEYWORDS.contains(&out.as_str()) { format!("{out}_") } else { out }
}

fn humanize(id: &str) -> String {
    let t = id.replace('_', " ");
    let mut c = t.trim().chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i];
        for j in 1..=b.len() {
            let c = usize::from(a[i - 1] != b[j - 1]);
            cur.push((prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + c));
        }
        prev = cur;
    }
    prev[b.len()]
}

fn words(s: &str) -> HashSet<String> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() >= 3).map(str::to_lowercase).collect()
}

/// Best existing id for a reference the model got slightly wrong.
fn fuzzy<'a>(want: &str, candidates: &[&'a str]) -> Option<&'a str> {
    let n = norm(want);
    if n.is_empty() {
        return None;
    }
    if let Some(c) = candidates.iter().find(|c| norm(c) == n) {
        return Some(c);
    }
    if n.len() >= 4 {
        let mut hits = candidates.iter().filter(|c| {
            let m = norm(c);
            m.len() >= 4 && (m.contains(&n) || n.contains(&m))
        });
        if let (Some(c), None) = (hits.next(), hits.next()) {
            return Some(c);
        }
    }
    if let Some((d, c)) = candidates.iter().map(|c| (edit_distance(&n, &norm(c)), *c)).min_by_key(|(d, _)| *d) {
        if d <= 2 && d * 4 <= n.len() {
            return Some(c);
        }
    }
    let ww = words(want);
    let mut scored: Vec<(f32, &str)> = candidates
        .iter()
        .map(|c| {
            let cw = words(c);
            let inter = ww.intersection(&cw).count() as f32;
            let uni = ww.union(&cw).count().max(1) as f32;
            (inter / uni, *c)
        })
        .filter(|(j, _)| *j >= 0.5)
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    scored.first().map(|x| x.1)
}

/// Repairs dangling scene / location / character references, scenes with
/// no way on and a missing ending. Returns one sentence per fix (shown as
/// warnings, not errors).
pub fn autofix(b: &mut Bible) -> Vec<String> {
    let mut notes = Vec::new();
    // Locations.
    for i in 0..b.scenes.len() {
        let loc = b.scenes[i].location.clone();
        if b.location(&loc).is_some() {
            continue;
        }
        let sid = b.scenes[i].id.clone();
        let ids: Vec<&str> = b.locations.iter().map(|l| l.id.as_str()).collect();
        if let Some(m) = fuzzy(&loc, &ids).map(str::to_owned) {
            notes.push(format!("scene `{sid}`: location `{loc}` -> `{m}`"));
            b.scenes[i].location = m;
        } else if loc.trim().is_empty() && !b.locations.is_empty() {
            let m = b.locations[0].id.clone();
            notes.push(format!("scene `{sid}`: no location, using `{m}`"));
            b.scenes[i].location = m;
        } else {
            let id = slug(&loc);
            if id.is_empty() {
                continue;
            }
            notes.push(format!("location `{id}` was not defined: stub created"));
            b.locations.push(LocationSpec { id: id.clone(), name: humanize(&id), visual: humanize(&id) });
            b.scenes[i].location = id;
        }
    }
    // Characters.
    for i in 0..b.scenes.len() {
        let sid = b.scenes[i].id.clone();
        let mut out: Vec<String> = Vec::new();
        for c in b.scenes[i].characters.clone() {
            let ids: Vec<&str> = b.characters.iter().map(|c| c.id.as_str()).collect();
            let mapped = if b.character(&c).is_some() {
                Some(c.clone())
            } else if let Some(m) = fuzzy(&c, &ids) {
                notes.push(format!("scene `{sid}`: character `{c}` -> `{m}`"));
                Some(m.to_owned())
            } else if let Some(ch) = b.characters.iter().find(|x| norm(&x.name) == norm(&c)) {
                notes.push(format!("scene `{sid}`: character `{c}` -> `{}`", ch.id));
                Some(ch.id.clone())
            } else {
                notes.push(format!("scene `{sid}`: unknown character `{c}` removed"));
                None
            };
            if let Some(m) = mapped {
                if !out.contains(&m) {
                    out.push(m);
                }
            }
        }
        b.scenes[i].characters = out;
    }
    // Scene links.
    let n0 = b.scenes.len();
    for i in 0..n0 {
        let from = b.scenes[i].clone();
        let mut next: Vec<String> = Vec::new();
        for n in &from.next {
            let target = if b.scene(n).is_some() {
                n.clone()
            } else {
                let ids: Vec<&str> =
                    b.scenes.iter().map(|s| s.id.as_str()).filter(|s| *s != from.id).collect();
                if let Some(m) = fuzzy(n, &ids).map(str::to_owned) {
                    notes.push(format!("scene `{}`: `next` `{n}` -> `{m}`", from.id));
                    m
                } else {
                    let id = slug(n);
                    if id.is_empty() {
                        continue;
                    }
                    if b.scene(&id).is_none() {
                        // The stub leads on to the scene that followed the
                        // referencing one, else to an ending.
                        let follow = b.scenes[..n0]
                            .get(i + 1)
                            .map(|s| s.id.clone())
                            .or_else(|| b.scenes[..n0].iter().find(|s| s.ending).map(|s| s.id.clone()));
                        notes.push(format!(
                            "scene `{id}` was referenced by `{}` but not defined: stub created",
                            from.id
                        ));
                        b.scenes.push(SceneOutline {
                            id: id.clone(),
                            title: humanize(&id),
                            location: from.location.clone(),
                            characters: from.characters.clone(),
                            summary: format!(
                                "Continuation of {}: {}",
                                from.id,
                                humanize(&id).to_lowercase()
                            ),
                            ending: follow.is_none(),
                            next: follow.into_iter().collect(),
                        });
                    }
                    id
                }
            };
            if !next.contains(&target) {
                next.push(target);
            }
        }
        b.scenes[i].next = next;
    }
    // Dead ends: continue to the next scene in order, or end there.
    for i in 0..b.scenes.len() {
        if !b.scenes[i].ending && b.scenes[i].next.is_empty() {
            let sid = b.scenes[i].id.clone();
            match b.scenes.get(i + 1).map(|s| s.id.clone()) {
                Some(n) => {
                    notes.push(format!("scene `{sid}` had no `next`: linked to `{n}`"));
                    b.scenes[i].next = vec![n];
                }
                None => {
                    notes.push(format!("scene `{sid}` had no `next`: marked as an ending"));
                    b.scenes[i].ending = true;
                }
            }
        }
    }
    notes
}
