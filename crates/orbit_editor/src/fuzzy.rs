//! Fuzzy matching for the command palette (home-made, no dependency).
//!
//! A query matches a candidate when its characters appear in order
//! (case and accents ignored). Matches are scored so that consecutive
//! characters, word starts and the start of the string win, and shorter
//! candidates are preferred. A query with several words (`ouv proj`) needs
//! every word to match.

/// Result of a successful match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FuzzyMatch {
    /// Higher is better.
    pub score: i32,
    /// Char indices (not bytes) of the candidate that matched, ascending, to
    /// highlight them.
    pub positions: Vec<usize>,
}

const BASE: i32 = 16;
const BONUS_START: i32 = 12;
const BONUS_WORD: i32 = 10;
const BONUS_CONSECUTIVE: i32 = 14;
const MAX_GAP_PENALTY: i32 = 6;

fn fold(c: char) -> char {
    let c = c.to_lowercase().next().unwrap_or(c);
    match c {
        'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' => 'a',
        'ç' => 'c',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ñ' => 'n',
        'ò' | 'ó' | 'ô' | 'ö' | 'õ' | 'œ' => 'o',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'ý' | 'ÿ' => 'y',
        other => other,
    }
}

/// Lowercases and strips the accents of `text` (one char in, one char out,
/// so char positions of the result match the original).
pub fn fold_str(text: &str) -> String {
    text.chars().map(fold).collect()
}

fn is_separator(c: char) -> bool {
    c.is_whitespace() || matches!(c, '-' | '_' | '/' | '.' | ':' | '(' | ')' | '\'' | '’')
}

/// Best alignment of one word (`query`, already folded) in `cand`.
fn match_word(query: &[char], cand: &[char], cand_orig: &[char]) -> Option<(i32, Vec<usize>)> {
    let (m, n) = (query.len(), cand.len());
    if m == 0 {
        return Some((0, Vec::new()));
    }
    if m > n {
        return None;
    }
    let bonus = |j: usize| -> i32 {
        if j == 0 {
            BONUS_START
        } else if is_separator(cand_orig[j - 1]) {
            BONUS_WORD
        } else if cand_orig[j].is_uppercase() && cand_orig[j - 1].is_lowercase() {
            BONUS_WORD / 2 // camelCase boundary
        } else {
            0
        }
    };

    // dp[i][j]: best score with query[i] matched at candidate position j,
    // and the position of query[i - 1] it came from.
    let mut dp: Vec<Vec<Option<(i32, usize)>>> = vec![vec![None; n]; m];
    for j in 0..n {
        if cand[j] == query[0] {
            let lead = (j as i32).min(MAX_GAP_PENALTY);
            dp[0][j] = Some((BASE + bonus(j) - lead, 0));
        }
    }
    for i in 1..m {
        for j in i..n {
            if cand[j] != query[i] {
                continue;
            }
            let mut best: Option<(i32, usize)> = None;
            for (k, cell) in dp[i - 1].iter().enumerate().take(j).skip(i - 1) {
                let Some((prev, _)) = *cell else {
                    continue;
                };
                let link = if k + 1 == j {
                    BONUS_CONSECUTIVE
                } else {
                    -((j - k - 1) as i32).min(MAX_GAP_PENALTY)
                };
                let score = prev + BASE + bonus(j) + link;
                if best.is_none_or(|(b, _)| score > b) {
                    best = Some((score, k));
                }
            }
            dp[i][j] = best;
        }
    }

    let (mut best_j, mut best_score) = (None, i32::MIN);
    for (j, cell) in dp[m - 1].iter().enumerate() {
        if let Some((score, _)) = *cell
            && score > best_score
        {
            best_score = score;
            best_j = Some(j);
        }
    }
    let mut j = best_j?;
    let mut positions = vec![0; m];
    for i in (0..m).rev() {
        positions[i] = j;
        j = dp[i][j]?.1;
    }
    Some((best_score, positions))
}

/// Matches `query` against `candidate`. `None` when it does not match.
/// An empty query matches everything with score 0.
pub fn fuzzy_match(query: &str, candidate: &str) -> Option<FuzzyMatch> {
    let cand_orig: Vec<char> = candidate.chars().collect();
    let cand: Vec<char> = cand_orig.iter().map(|&c| fold(c)).collect();
    let mut total = 0;
    let mut positions = Vec::new();
    for word in query.split_whitespace() {
        let q: Vec<char> = word.chars().map(fold).collect();
        let (score, pos) = match_word(&q, &cand, &cand_orig)?;
        total += score;
        positions.extend(pos);
    }
    positions.sort_unstable();
    positions.dedup();
    // Prefer shorter candidates among equal matches.
    if !positions.is_empty() {
        total -= (cand.len() as i32) / 4;
    }
    Some(FuzzyMatch {
        score: total,
        positions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranked<'a>(query: &str, items: &[&'a str]) -> Vec<&'a str> {
        let mut hits: Vec<(i32, usize, &str)> = items
            .iter()
            .enumerate()
            .filter_map(|(i, s)| fuzzy_match(query, s).map(|m| (m.score, i, *s)))
            .collect();
        hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        hits.into_iter().map(|(_, _, s)| s).collect()
    }

    #[test]
    fn empty_query_matches_everything() {
        let m = fuzzy_match("", "Enregistrer").unwrap();
        assert_eq!(m.score, 0);
        assert!(m.positions.is_empty());
    }

    #[test]
    fn non_subsequence_does_not_match() {
        assert!(fuzzy_match("xyz", "Enregistrer").is_none());
        assert!(fuzzy_match("rr", "Ouvrir").is_some());
        assert!(fuzzy_match("rrr", "Ouvrir").is_none());
        assert!(fuzzy_match("abcdefghij", "abc").is_none());
    }

    #[test]
    fn case_and_accents_are_ignored() {
        assert!(fuzzy_match("ECRAN", "Écran").is_some());
        assert!(fuzzy_match("eleve", "Élève").is_some());
        assert!(fuzzy_match("facon", "Façon").is_some());
    }

    #[test]
    fn prefix_beats_scattered_match() {
        let order = ranked(
            "ou",
            &["Fermer le projet", "Nouveau projet", "Ouvrir un projet"],
        );
        assert_eq!(order[0], "Ouvrir un projet");
        assert_eq!(order[1], "Nouveau projet");
        assert_eq!(order.len(), 2); // "Fermer le projet" has no `o` before a `u`
    }

    #[test]
    fn word_starts_win_over_inner_letters() {
        let order = ranked("np", &["Ouvrir un projet", "Nouveau projet"]);
        assert_eq!(order[0], "Nouveau projet");
    }

    #[test]
    fn consecutive_run_beats_spread_match() {
        let order = ranked("sav", &["Sélectionner la valeur", "Sauvegarder"]);
        assert_eq!(order[0], "Sauvegarder");
    }

    #[test]
    fn shorter_candidate_wins_on_equal_match() {
        let order = ranked("pan", &["Panneau de configuration", "Panneau"]);
        assert_eq!(order[0], "Panneau");
    }

    #[test]
    fn several_words_all_have_to_match() {
        assert!(fuzzy_match("ouv proj", "Ouvrir un projet").is_some());
        assert!(fuzzy_match("ouv xyz", "Ouvrir un projet").is_none());
        let order = ranked(
            "aff con",
            &["Afficher la console", "Afficher l'explorateur"],
        );
        assert_eq!(order, ["Afficher la console"]);
    }

    #[test]
    fn positions_are_char_indices_for_highlighting() {
        let m = fuzzy_match("ec", "Écran").unwrap();
        assert_eq!(m.positions, vec![0, 1]);
        let m = fuzzy_match("np", "Nouveau projet").unwrap();
        assert_eq!(m.positions, vec![0, 8]);
    }
}
