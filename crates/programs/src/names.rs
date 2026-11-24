//! Names as they are compared: how far one is from another in keystrokes,
//! and a program's name without what a system adds to it.

/// How many single-character edits turn `a` into `b`: insert, delete,
/// replace, or swap two neighbors (optimal string alignment distance).
#[must_use]
pub fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let width = b.len() + 1;
    let mut table = vec![0usize; (a.len() + 1) * width];
    let at = |i: usize, j: usize| i * width + j;
    for i in 0..=a.len() {
        table[at(i, 0)] = i;
    }
    for j in 0..=b.len() {
        table[at(0, j)] = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (table[at(i - 1, j)] + 1).min(table[at(i, j - 1)] + 1).min(table[at(i - 1, j - 1)] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(table[at(i - 2, j - 2)] + 1);
            }
            table[at(i, j)] = best;
        }
    }
    table[at(a.len(), b.len())]
}

/// A program's name as compared: no `.exe` or the like, lower case.
#[must_use]
pub fn bare(name: &str) -> String {
    let lower = name.to_lowercase();
    for extension in [".exe", ".cmd", ".bat", ".com", ".ps1"] {
        if let Some(stem) = lower.strip_suffix(extension) {
            return stem.to_owned();
        }
    }
    lower
}

/// How many keystrokes a mistyped name may be from a known one to be taken
/// for it: one for names of up to four letters, two for longer ones.
#[must_use]
pub fn within_reach(typed: &str) -> usize {
    if typed.chars().count() <= 4 { 1 } else { 2 }
}

/// The name in `known` nearest to `typed` and within reach of it, with how
/// far it is. Of names as near, the first wins. `None` when nothing is near.
pub fn nearest<'a>(typed: &str, known: impl IntoIterator<Item = &'a str>) -> Option<(&'a str, usize)> {
    let allowed = within_reach(typed);
    known
        .into_iter()
        .map(|candidate| (candidate, distance(typed, candidate)))
        .filter(|(_, how_far)| *how_far <= allowed)
        .min_by_key(|(_, how_far)| *how_far)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_distance_counts_swaps_as_one() {
        assert_eq!(distance("gti", "git"), 1, "a swap");
        assert_eq!(distance("git", "git"), 0, "the same");
        assert_eq!(distance("", "abc"), 3, "all inserted");
        assert_eq!(distance("kitten", "sitting"), 3, "the textbook pair");
        assert_eq!(distance("dokcer", "docker"), 1, "a swap inside");
        assert_eq!(distance("中文", "中午"), 1, "characters, not bytes");
    }

    #[test]
    fn the_nearest_name_is_the_first_of_those_as_near() {
        assert_eq!(nearest("splt-window", ["split-window", "swap-window"]), Some(("split-window", 1)));
        assert_eq!(nearest("ab", ["ax", "ay"]), Some(("ax", 1)), "the first of two as near");
        assert_eq!(nearest("xx", ["ls", "cd"]), None, "two edits is too far for a short name");
        assert_eq!(nearest("kubctel", ["kubectl"]), Some(("kubectl", 2)), "two edits for a long name");
        assert_eq!(bare("Git.EXE"), "git");
        assert_eq!(bare("build.sh"), "build.sh");
    }
}
