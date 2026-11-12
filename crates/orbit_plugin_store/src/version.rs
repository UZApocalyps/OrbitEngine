//! Plugin versions: `major.minor.patch[-pre][+build]` (semver order; the
//! build suffix is ignored, a pre-release sorts before its release).

use std::cmp::Ordering;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub core: [u64; 3],
    pub pre: Vec<String>,
}

impl Version {
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        let text = text.split('+').next()?;
        let (core, pre) = match text.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (text, None),
        };
        let mut parts = [0u64; 3];
        let mut n = 0;
        for part in core.split('.') {
            if n >= 3 || part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            parts[n] = part.parse().ok()?;
            n += 1;
        }
        if n == 0 {
            return None;
        }
        let pre = match pre {
            Some("") => return None,
            Some(p) => p.split('.').map(str::to_owned).collect(),
            None => Vec::new(),
        };
        Some(Self { core: parts, pre })
    }
}

fn cmp_ident(a: &str, b: &str) -> Ordering {
    match (a.parse::<u64>(), b.parse::<u64>()) {
        (Ok(x), Ok(y)) => x.cmp(&y),
        (Ok(_), Err(_)) => Ordering::Less,
        (Err(_), Ok(_)) => Ordering::Greater,
        _ => a.cmp(b),
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core
            .cmp(&other.core)
            .then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                _ => {
                    for (a, b) in self.pre.iter().zip(&other.pre) {
                        let o = cmp_ident(a, b);
                        if o != Ordering::Equal {
                            return o;
                        }
                    }
                    self.pre.len().cmp(&other.pre.len())
                }
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Compares two version texts; unparsable ones sort first (and equal).
pub fn compare(a: &str, b: &str) -> Ordering {
    match (Version::parse(a), Version::parse(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Greater,
        (None, Some(_)) => Ordering::Less,
        (None, None) => Ordering::Equal,
    }
}

/// `candidate` is strictly newer than `installed`.
pub fn is_newer(candidate: &str, installed: &str) -> bool {
    matches!((Version::parse(candidate), Version::parse(installed)), (Some(c), Some(i)) if c > i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_order_like_semver() {
        let order = [
            "0.1.0",
            "0.3.1-alpha",
            "0.3.1-alpha.2",
            "0.3.1-alpha.10",
            "0.3.1-beta",
            "0.3.1",
            "0.3.2",
            "1.0.0",
        ];
        for w in order.windows(2) {
            assert_eq!(compare(w[0], w[1]), Ordering::Less, "{} < {}", w[0], w[1]);
        }
        assert_eq!(compare("1.2.3+build5", "1.2.3"), Ordering::Equal);
        assert_eq!(compare("v1.2", "1.2.0"), Ordering::Equal);
        assert!(is_newer("0.4.1", "0.4.0"));
        assert!(!is_newer("0.4.0", "0.4.0"));
        assert!(!is_newer("garbage", "0.1.0"));
        for bad in ["", "1..2", "1.2.3.4", "a.b", "1.2-", "-1"] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
    }
}
