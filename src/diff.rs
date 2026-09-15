//! A minimal line diff, so E42 can report churn as a number.
//!
//! Plain LCS. Snapshot files here are on the order of a thousand lines, so the
//! quadratic table is a few megabytes and nobody needs Myers.

/// How much of a golden file a change disturbs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffStat {
    pub lines_before: usize,
    pub lines_after: usize,
    pub added: usize,
    pub removed: usize,
}

impl DiffStat {
    pub fn touched(&self) -> usize {
        self.added + self.removed
    }

    /// Churn as a percentage of the larger file. This is the number that decides
    /// whether a text golden is reviewable in a pull request.
    pub fn churn_percent(&self) -> f64 {
        let denom = self.lines_before.max(self.lines_after).max(1) as f64;
        (self.touched() as f64 / denom) * 100.0
    }
}

impl core::fmt::Display for DiffStat {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "+{} -{} of {} lines ({:.2}% churn)",
            self.added,
            self.removed,
            self.lines_before.max(self.lines_after),
            self.churn_percent()
        )
    }
}

/// Length of the longest common subsequence of the two line slices.
fn lcs_len(a: &[&str], b: &[&str]) -> usize {
    // Two rolling rows instead of the full table: the length is all we need.
    let mut prev = vec![0u32; b.len() + 1];
    let mut cur = vec![0u32; b.len() + 1];
    for i in 0..a.len() {
        cur[0] = 0;
        for j in 0..b.len() {
            cur[j + 1] = if a[i] == b[j] {
                prev[j] + 1
            } else {
                cur[j].max(prev[j + 1])
            };
        }
        core::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()] as usize
}

pub fn diff(before: &str, after: &str) -> DiffStat {
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();
    let common = lcs_len(&a, &b);
    DiffStat {
        lines_before: a.len(),
        lines_after: b.len(),
        removed: a.len() - common,
        added: b.len() - common,
    }
}
